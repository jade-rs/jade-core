//! Styled OMML plans for auxiliary stories. The context supplies glyph metrics
//! and script scaling; stretchy MATH glyph variants remain separate. Fallback geometry
//! is explicit and does not turn structure into preview punctuation.
use super::{find_child, is_property_tag, nary_symbol, MathMetrics, MathParagraphAlignment};
use jade_docx_reader::document::{MathElement, MathElementKind, MathParagraph};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[cfg(feature = "opentype-math")]
pub mod font;

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
    pub style: MathTextStyle<'a>,
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
#[derive(Debug)]
pub struct StyledMathLayout<'a> {
    pub metrics: MathMetrics,
    /// Required paragraph box; ink geometry and glyph baselines stay separate.
    #[cfg(feature = "opentype-math")]
    pub line_metrics: MathMetrics,
    pub alignment: MathParagraphAlignment,
    pub text: String,
    pub glyphs: Vec<MathGlyph<'a>>,
    pub rules: Vec<MathRule<'a>>,
}

pub fn layout_styled_math<'a, C: StyledMathMeasure>(
    context: &C,
    math: &'a MathParagraph,
    style: MathTextStyle<'a>,
) -> Result<StyledMathLayout<'a>, C::Error> {
    let mut builder = Builder {
        context,
        style,
        level: 0,
        #[cfg(feature = "opentype-math")]
        display_operator: false,
        plan: StyledMathLayout {
            metrics: MathMetrics::default(),
            #[cfg(feature = "opentype-math")]
            line_metrics: MathMetrics::default(),
            alignment: math
                .elements
                .iter()
                .find_map(MathParagraphAlignment::from_element)
                .unwrap_or(MathParagraphAlignment::Center),
            text: String::new(),
            glyphs: Vec::new(),
            rules: Vec::new(),
        },
    };
    builder.plan.metrics = builder.elements(&math.elements)?;
    #[cfg(feature = "opentype-math")]
    {
        builder.plan.line_metrics = context
            .font_parameters(style)
            .map_or(builder.plan.metrics, |font| {
                font.line_metrics(builder.plan.metrics)
            });
    }
    Ok(builder.plan)
}

struct Builder<'c, 'a, C> {
    context: &'c C,
    style: MathTextStyle<'a>,
    level: u32,
    #[cfg(feature = "opentype-math")]
    display_operator: bool,
    plan: StyledMathLayout<'a>,
}
#[derive(Clone, Copy)]
struct Mark {
    glyphs: usize,
    rules: usize,
}
#[cfg(feature = "opentype-math")]
struct FontBase {
    start: Mark,
    end: Mark,
    metrics: MathMetrics,
    parameters: font::MathFontParameters,
    italic: f64,
    size_px: f64,
}
#[cfg(feature = "opentype-math")]
struct ScriptBox {
    start: Mark,
    end: Mark,
    metrics: MathMetrics,
}
impl<'a, C: StyledMathMeasure> Builder<'_, 'a, C> {
    fn mark(&self) -> Mark {
        Mark {
            glyphs: self.plan.glyphs.len(),
            rules: self.plan.rules.len(),
        }
    }
    fn shift(&mut self, start: Mark, x: f64, y: f64) {
        for g in &mut self.plan.glyphs[start.glyphs..] {
            g.x += x;
            g.baseline += y;
        }
        for r in &mut self.plan.rules[start.rules..] {
            r.x += x;
            r.y += y;
        }
    }
    fn text(&mut self, text: &str) -> Result<MathMetrics, C::Error> {
        let mut total = MathMetrics::default();
        let mut style = self.style;
        style.size_px *= self.context.script_scale(style, self.level);
        for grapheme in text.graphemes(true) {
            let start = self.plan.text.len();
            for ch in grapheme.chars() {
                self.plan.text.push(math_alphabetic(ch, style.variant));
            }
            let range = start..self.plan.text.len();
            #[cfg(feature = "opentype-math")]
            let font_glyph = self.context.prepare_glyph_at_script_level(
                &self.plan.text[range.clone()],
                style,
                self.display_operator,
                self.level,
            )?;
            #[cfg(feature = "opentype-math")]
            let metrics = match &font_glyph {
                Some(glyph) => glyph.metrics(),
                None => self
                    .context
                    .measure_glyph(&self.plan.text[range.clone()], style)?,
            };
            #[cfg(not(feature = "opentype-math"))]
            let metrics = self
                .context
                .measure_glyph(&self.plan.text[range.clone()], style)?;
            let x = total.width;
            self.plan.glyphs.push(MathGlyph {
                text_range: range,
                x,
                baseline: 0.0,
                width: metrics.width,
                style,
                #[cfg(feature = "opentype-math")]
                font_glyph,
            });
            total.include(metrics, x, 0.0);
        }
        Ok(total)
    }
    fn elements(&mut self, elements: &'a [MathElement]) -> Result<MathMetrics, C::Error> {
        let mut total = MathMetrics::default();
        for e in elements {
            if e.tag.as_deref().is_some_and(is_property_tag) {
                continue;
            }
            let start = self.mark();
            let metrics = self.element(e)?;
            let x = total.width;
            self.shift(start, x, 0.0);
            total.include(metrics, x, 0.0);
        }
        Ok(total)
    }
    fn child(&mut self, e: Option<&'a MathElement>, script: bool) -> Result<MathMetrics, C::Error> {
        if script {
            self.level += 1;
        }
        let result = e
            .map(|e| self.element(e))
            .transpose()
            .map(Option::unwrap_or_default);
        if script {
            self.level -= 1;
        }
        result
    }
    fn element(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        match e.kind {
            Run => {
                let previous = self.style;
                self.style = run_style(e, previous);
                let result = self.elements(&e.children);
                self.style = previous;
                result
            }
            Text => self.text(e.text.as_deref().unwrap_or_default()),
            Superscript | Subscript => {
                #[cfg(feature = "opentype-math")]
                let base_start = self.mark();
                let mut total = self.child(find_child(e, Base), false)?;
                #[cfg(feature = "opentype-math")]
                if let Some(base) = self.font_base(base_start, total) {
                    return self.font_scripts(e, base, false, true);
                }
                let x = total.width;
                let start = self.mark();
                let superscript = e.kind == Superscript;
                let metrics = self.child(
                    find_child(
                        e,
                        if superscript {
                            SuperArgument
                        } else {
                            SubArgument
                        },
                    ),
                    true,
                )?;
                let y = self.style.size_px * if superscript { -0.6 } else { 0.4 };
                self.shift(start, x, y);
                total.include(metrics, x, y);
                Ok(total)
            }
            Nary => self.nary(e),
            Fraction => self.fraction(e),
            Delimiter => {
                let pr = e.children.iter().find(|c| c.tag.as_deref() == Some("dPr"));
                let begin = property(pr, "begChr").unwrap_or("(");
                let end = property(pr, "endChr").unwrap_or(")");
                let mut total = self.text(begin)?;
                for child in e.children.iter().filter(|c| c.kind == Base) {
                    let start = self.mark();
                    let metrics = self.element(child)?;
                    let x = total.width;
                    self.shift(start, x, 0.0);
                    total.include(metrics, x, 0.0);
                }
                let start = self.mark();
                let metrics = self.text(end)?;
                let x = total.width;
                self.shift(start, x, 0.0);
                total.include(metrics, x, 0.0);
                Ok(total)
            }
            Radical => {
                let mut total = self.text("√")?;
                let start = self.mark();
                let metrics = self.child(find_child(e, Base), false)?;
                let x = total.width;
                self.shift(start, x, 0.0);
                total.include(metrics, x, 0.0);
                Ok(total)
            }
            #[cfg(feature = "opentype-math")]
            Unknown if e.tag.as_deref() == Some("sSubSup") => {
                // Only structured base-first scripts enter the optional path;
                // preserve the original traversal for unsupported structures.
                let mut children = e
                    .children
                    .iter()
                    .filter(|c| !c.tag.as_deref().is_some_and(is_property_tag));
                if children.next().is_none_or(|c| c.kind != Base)
                    || children.any(|c| c.kind == Base)
                {
                    return self.elements(&e.children);
                }
                let start = self.mark();
                let base = self.child(find_child(e, Base), false)?;
                if let Some(base) = self.font_base(start, base) {
                    self.font_scripts(e, base, false, true)
                } else {
                    // Retain the original unbound structural fallback without
                    // measuring or appending its base a second time.
                    let mut total = base;
                    for child in e.children.iter().filter(|c| {
                        c.kind != Base && !c.tag.as_deref().is_some_and(is_property_tag)
                    }) {
                        let start = self.mark();
                        let metrics = self.element(child)?;
                        let x = total.width;
                        self.shift(start, x, 0.0);
                        total.include(metrics, x, 0.0);
                    }
                    Ok(total)
                }
            }
            // Containers retain their children. Unsupported stretchy constructs
            // still need font-specific variants; do not invent preview labels.
            _ => self.elements(&e.children),
        }
    }
    fn nary(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        #[cfg(feature = "opentype-math")]
        let base_start = self.mark();
        let symbol = nary_symbol(e).unwrap_or_else(|| "∫".to_string());
        #[cfg(feature = "opentype-math")]
        let mut total = {
            self.display_operator = true;
            let result = self.text(&symbol);
            self.display_operator = false;
            result?
        };
        #[cfg(not(feature = "opentype-math"))]
        let mut total = self.text(&symbol)?;
        let symbol_width = total.width;
        let pr = e
            .children
            .iter()
            .find(|c| c.tag.as_deref() == Some("naryPr"));
        let side = property(pr, "limLoc") == Some("subSup");
        #[cfg(feature = "opentype-math")]
        if let Some(base) = self.font_base(base_start, total) {
            return self.font_scripts(e, base, true, side);
        }
        let mut right = symbol_width;
        for (kind, y) in [
            (SuperArgument, -self.style.size_px * 0.8),
            (SubArgument, self.style.size_px * 0.5),
        ] {
            let start = self.mark();
            let metrics = self.child(find_child(e, kind), true)?;
            let x = if side {
                symbol_width
            } else {
                ((symbol_width - metrics.width) / 2.0).max(0.0)
            };
            self.shift(start, x, y);
            total.include(metrics, x, y);
            right = right.max(x + metrics.width);
        }
        let start = self.mark();
        let metrics = self.child(find_child(e, Base), false)?;
        let x = right + self.style.size_px * 0.1;
        self.shift(start, x, 0.0);
        total.include(metrics, x, 0.0);
        Ok(total)
    }
    #[cfg(feature = "opentype-math")]
    fn font_base(&self, start: Mark, metrics: MathMetrics) -> Option<FontBase> {
        let end = self.mark();
        let glyphs = &self.plan.glyphs[start.glyphs..end.glyphs];
        let first = glyphs.first()?;
        let prepared = first.font_glyph.as_ref()?;
        if glyphs.iter().any(|g| g.style.family != first.style.family) {
            return None;
        }
        Some(FontBase {
            start,
            end,
            metrics,
            parameters: self.context.font_parameters(first.style)?,
            italic: if glyphs.len() == 1 && start.rules == end.rules {
                prepared.italic_correction_px
            } else {
                0.0
            },
            size_px: first.style.size_px,
        })
    }
    #[cfg(feature = "opentype-math")]
    fn script_box(
        &mut self,
        argument: Option<&'a MathElement>,
    ) -> Result<Option<ScriptBox>, C::Error> {
        let Some(argument) = argument else {
            return Ok(None);
        };
        let start = self.mark();
        let metrics = self.child(Some(argument), true)?;
        Ok(Some(ScriptBox {
            start,
            end: self.mark(),
            metrics,
        }))
    }
    #[cfg(feature = "opentype-math")]
    fn shift_range(&mut self, start: Mark, end: Mark, x: f64, y: f64) {
        for glyph in &mut self.plan.glyphs[start.glyphs..end.glyphs] {
            glyph.x += x;
            glyph.baseline += y;
        }
        for rule in &mut self.plan.rules[start.rules..end.rules] {
            rule.x += x;
            rule.y += y;
        }
    }
    #[cfg(feature = "opentype-math")]
    fn font_scripts(
        &mut self,
        e: &'a MathElement,
        base: FontBase,
        nary: bool,
        side: bool,
    ) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let pr = e
            .children
            .iter()
            .find(|c| c.tag.as_deref() == Some("naryPr"));
        let upper = if e.kind == Subscript || (nary && on_property(pr, "supHide")) {
            None
        } else {
            find_child(e, SuperArgument)
        };
        let lower = if e.kind == Superscript || (nary && on_property(pr, "subHide")) {
            None
        } else {
            find_child(e, SubArgument)
        };
        // Preserve the semantic traversal used by each existing structure.
        let (upper, lower) = if nary {
            let upper = self.script_box(upper)?;
            let lower = self.script_box(lower)?;
            (upper, lower)
        } else {
            let lower = self.script_box(lower)?;
            let upper = self.script_box(upper)?;
            (upper, lower)
        };
        let shifts = if side {
            base.parameters.script_shifts(
                base.metrics,
                upper.as_ref().map(|s| s.metrics),
                lower.as_ref().map(|s| s.metrics),
            )
        } else {
            base.parameters.limit_shifts(
                base.metrics,
                upper.as_ref().map(|s| s.metrics),
                lower.as_ref().map(|s| s.metrics),
            )
        };
        let upper_x = upper.as_ref().map_or(0.0, |s| {
            if side {
                base.metrics.width + if nary { 0.0 } else { base.italic }
            } else {
                (base.metrics.width - s.metrics.width) / 2.0 + base.italic / 2.0
            }
        });
        let lower_x = lower.as_ref().map_or(0.0, |s| {
            if side {
                base.metrics.width - if nary { base.italic } else { 0.0 }
            } else {
                (base.metrics.width - s.metrics.width) / 2.0 - base.italic / 2.0
            }
        });
        // Long limits may extend before the operator. Include that extent in
        // the shared advance instead of clipping/centering them in a painter.
        let origin = -0.0_f64.min(upper_x).min(lower_x);
        self.shift_range(base.start, base.end, origin, 0.0);
        let mut total = MathMetrics::default();
        total.include(base.metrics, origin, 0.0);
        if let Some(s) = &upper {
            self.shift_range(s.start, s.end, upper_x + origin, -shifts.up);
            total.include(s.metrics, upper_x + origin, -shifts.up);
        }
        if let Some(s) = &lower {
            self.shift_range(s.start, s.end, lower_x + origin, shifts.down);
            total.include(s.metrics, lower_x + origin, shifts.down);
        }
        if side && (upper.is_some() || lower.is_some()) {
            total.width += base.parameters.space_after_script();
        }
        if nary {
            let start = self.mark();
            let body = self.child(find_child(e, Base), false)?;
            // Operator-to-body spacing is still the existing fallback policy;
            // SpaceAfterScript above is separate, font-defined whitespace.
            let x = total.width + base.size_px * 0.1;
            self.shift(start, x, 0.0);
            total.include(body, x, 0.0);
        }
        Ok(total)
    }
    fn fraction(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let num_start = self.mark();
        let numerator = self.child(find_child(e, Numerator), false)?;
        let num_end = self.mark();
        let denominator = self.child(find_child(e, Denominator), false)?;
        let padding = self.style.size_px * 0.2;
        let thickness = (self.style.size_px / 16.0).max(0.5);
        let width = numerator.width.max(denominator.width) + 2.0 * padding;
        let up = -(numerator.descent + thickness + self.style.size_px * 0.15);
        let down = denominator.ascent + thickness + self.style.size_px * 0.15;
        // Restrict numerator translation to its commands before denominator.
        for g in &mut self.plan.glyphs[num_start.glyphs..num_end.glyphs] {
            g.x += (width - numerator.width) / 2.0;
            g.baseline += up;
        }
        for r in &mut self.plan.rules[num_start.rules..num_end.rules] {
            r.x += (width - numerator.width) / 2.0;
            r.y += up;
        }
        self.shift(num_end, (width - denominator.width) / 2.0, down);
        self.plan.rules.push(MathRule {
            x: padding,
            y: 0.0,
            width: width - 2.0 * padding,
            thickness,
            color: self.style.color,
        });
        let mut total = MathMetrics {
            width,
            ascent: thickness / 2.0,
            descent: thickness / 2.0,
        };
        total.include(numerator, 0.0, up);
        total.include(denominator, 0.0, down);
        Ok(total)
    }
}
fn property<'a>(element: Option<&'a MathElement>, name: &str) -> Option<&'a str> {
    element?
        .children
        .iter()
        .find(|c| c.tag.as_deref() == Some(name))?
        .attributes
        .get("val")
        .map(String::as_str)
}
#[cfg(feature = "opentype-math")]
fn on_property(element: Option<&MathElement>, name: &str) -> bool {
    element
        .and_then(|pr| pr.children.iter().find(|c| c.tag.as_deref() == Some(name)))
        .is_some_and(|p| {
            p.attributes
                .get("val")
                .is_none_or(|v| matches!(v.as_str(), "1" | "true" | "on"))
        })
}
fn run_style<'a>(run: &'a MathElement, mut style: MathTextStyle<'a>) -> MathTextStyle<'a> {
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
                Some("nor")
                    if !matches!(
                        p.attributes.get("val").map(String::as_str),
                        Some("0" | "false" | "off")
                    ) =>
                {
                    style.variant = MathVariant::Plain
                }
                _ => {}
            }
        }
    }
    style
}
fn math_alphabetic(ch: char, variant: MathVariant) -> char {
    use MathVariant::*;
    let base = match (variant, ch) {
        (Italic, 'h') => return 'ℎ',
        (Italic, 'A'..='Z') => Some((0x1D434, 'A')),
        (Italic, 'a'..='z') => Some((0x1D44E, 'a')),
        (Bold, 'A'..='Z') => Some((0x1D400, 'A')),
        (Bold, 'a'..='z') => Some((0x1D41A, 'a')),
        (BoldItalic, 'A'..='Z') => Some((0x1D468, 'A')),
        (BoldItalic, 'a'..='z') => Some((0x1D482, 'a')),
        (Bold | BoldItalic, '0'..='9') => Some((0x1D7CE, '0')),
        _ => None,
    };
    base.and_then(|(start, first)| char::from_u32(start + ch as u32 - first as u32))
        .unwrap_or(ch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    #[derive(Default)]
    struct Measure {
        calls: RefCell<Vec<(String, f64)>>,
    }
    impl StyledMathMeasure for Measure {
        type Error = &'static str;
        fn measure_glyph(
            &self,
            text: &str,
            style: MathTextStyle<'_>,
        ) -> Result<MathMetrics, Self::Error> {
            self.calls
                .borrow_mut()
                .push((text.to_string(), style.size_px));
            if text == "!" {
                return Err("unavailable");
            }
            Ok(MathMetrics {
                width: style.size_px * 0.5,
                ascent: style.size_px * 0.8,
                descent: style.size_px * 0.2,
            })
        }
    }
    fn parse(body: &str) -> MathParagraph {
        jade_docx_reader::parser::math::parse_math_paragraph(format!(r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{body}</m:oMath>"#)).unwrap()
    }
    fn style() -> MathTextStyle<'static> {
        MathTextStyle {
            family: "Cambria Math",
            size_px: 14.0,
            color: "#000000",
            variant: MathVariant::Italic,
        }
    }
    #[test]
    fn styled_runs_preserve_authored_font_color_and_unicode_math_alphabetics() {
        let math = parse(
            r#"<m:r><w:rPr><w:rFonts w:ascii="Other Math"/><w:sz w:val="18"/><w:color w:val="112233"/></w:rPr><m:rPr><m:sty m:val="bi"/></m:rPr><m:t>Az2</m:t></m:r><m:r><m:rPr><m:nor/></m:rPr><m:t>xy</m:t></m:r><m:r><m:t>h</m:t></m:r>"#,
        );
        let m = Measure::default();
        let p = layout_styled_math(&m, &math, style()).unwrap();
        assert_eq!(p.text, "𝑨𝒛𝟐xyℎ");
        assert_eq!(m.calls.borrow().len(), p.glyphs.len());
        assert_eq!(p.glyphs[0].style.family, "Other Math");
        assert_eq!(p.glyphs[0].style.size_px, 12.0);
        assert_eq!(p.glyphs[0].style.color, "112233");
        assert_eq!(p.glyphs[3].style.size_px, 14.0);
        for g in &p.glyphs {
            assert!(!p.text[g.text_range.clone()].is_empty());
        }
    }
    #[test]
    fn structured_nary_delimiters_and_scripts_have_measured_utf8_glyph_ranges() {
        let math = parse(
            r#"<m:r><m:t>P</m:t></m:r><m:d><m:e><m:r><m:t>x</m:t></m:r></m:e></m:d><m:nary><m:naryPr><m:chr m:val="∑"/><m:limLoc m:val="subSup"/></m:naryPr><m:sub><m:r><m:t>a</m:t></m:r></m:sub><m:sup><m:r><m:t>b</m:t></m:r></m:sup><m:e><m:r><m:t>f</m:t></m:r></m:e></m:nary>"#,
        );
        let m = Measure::default();
        let p = layout_styled_math(&m, &math, style()).unwrap();
        assert_eq!(p.text, "𝑃(𝑥)∑𝑏𝑎𝑓");
        let glyph = |text: &str| {
            p.glyphs
                .iter()
                .find(|g| &p.text[g.text_range.clone()] == text)
                .unwrap()
        };
        assert!(glyph("𝑏").baseline < glyph("∑").baseline);
        assert!(glyph("𝑎").baseline > glyph("∑").baseline);
        assert_eq!(glyph("𝑏").style.size_px, 14.0 * 0.8);
        assert!(glyph("𝑏").x > glyph("∑").x);
        assert_eq!(m.calls.borrow().len(), p.glyphs.len());
        for g in &p.glyphs {
            assert!(g.x >= 0.0 && g.x + g.width <= p.metrics.width + 1e-9);
            assert!(g.baseline - g.style.size_px * 0.8 >= -p.metrics.ascent - 1e-9);
            assert!(g.baseline + g.style.size_px * 0.2 <= p.metrics.descent + 1e-9);
        }
    }
    #[test]
    fn styled_fraction_rules_and_children_fit_bounds_and_errors_propagate() {
        let math=parse("<m:f><m:num><m:r><m:t>ab</m:t></m:r></m:num><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f>");
        let p = layout_styled_math(&Measure::default(), &math, style()).unwrap();
        assert_eq!(p.rules.len(), 1);
        assert!(p.glyphs[0].baseline < 0.0 && p.glyphs[2].baseline > 0.0);
        for g in &p.glyphs {
            assert!(g.x >= 0.0 && g.x + g.width <= p.metrics.width + 1e-9);
        }
        assert_eq!(
            layout_styled_math(
                &Measure::default(),
                &parse("<m:r><m:t>!</m:t></m:r>"),
                style()
            )
            .unwrap_err(),
            "unavailable"
        );
    }
}
