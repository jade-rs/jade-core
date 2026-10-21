//! Baseline-relative mathematical layout consumed by document renderers.
//!
//! The fallback geometry retains the existing math renderer's placement rules.
//! Font-specific OpenType MATH shaping remains separate work; this plan makes
//! measurement and painting use one geometry and measures each emitted text once.

pub mod eq_field;
#[cfg(feature = "opentype-math")]
pub mod font;
pub mod linear;
pub mod styled;

use jade_docx_reader::document::{MathElement, MathElementKind, MathParagraph};
use std::borrow::Cow;
use std::ops::Range;

/// The first mathematical line uses the enclosing paragraph's indented box.
/// Shared paragraph resolvers retain logical/character/mirrored precedence.
pub fn math_block_content_box(
    properties: Option<&jade_docx_reader::ParagraphProperties>,
    column_left: f64,
    column_width: f64,
    char_unit_pt: f64,
    page_is_even: bool,
) -> (f64, f64) {
    use crate::line_wrap::{
        mirror_indent_for_page, resolve_indent_first_line_pt, resolve_indent_left_pt,
        resolve_indent_right_pt,
    };
    use crate::units::points_to_px;
    let Some(properties) = properties else {
        return (column_left, column_width.max(1.0));
    };
    let Some(indent) = &properties.indent else {
        return (column_left, column_width.max(1.0));
    };
    let bidi = properties.bidi.unwrap_or(false);
    let indent = mirror_indent_for_page(
        indent,
        bidi,
        properties.mirror_indents.unwrap_or(false),
        page_is_even,
    );
    let px = |value: Option<f64>| points_to_px(value.unwrap_or(0.0) as f32);
    let left = px(resolve_indent_left_pt(&indent, false, char_unit_pt));
    let right = px(resolve_indent_right_pt(&indent, false, char_unit_pt));
    let first = px(resolve_indent_first_line_pt(
        &indent,
        char_unit_pt,
        properties.numbering.is_none(),
    ));
    (
        column_left + left + first,
        (column_width - left - right - first).max(1.0),
    )
}

/// Line height and baseline from a shared math plan and ordinary paragraph rules.
/// Pass the plan's paragraph metrics, including font MATH leading and its floor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MathBlockLineBox {
    pub height: f64,
    pub baseline: f64,
}

pub fn math_block_line_box(
    metrics: MathMetrics,
    font_size_pt: f32,
    properties: Option<&jade_docx_reader::ParagraphProperties>,
    grid: Option<&jade_docx_reader::DocGrid>,
    document_snap_to_grid: bool,
    section_content_height_pt: Option<f32>,
) -> MathBlockLineBox {
    use crate::line_metrics::{
        resolve_effective_snap_to_grid, resolve_line_height_points, word_grid_line_centres_text,
        WordGridLineSizing,
    };
    use crate::units::points_to_px;
    let spacing = properties.and_then(|p| p.spacing.as_ref());
    let snap = resolve_effective_snap_to_grid(
        properties.and_then(|p| p.snap_to_grid),
        document_snap_to_grid,
        grid,
    );
    let natural_height = metrics.ascent + metrics.descent;
    let mut height = points_to_px(resolve_line_height_points(
        font_size_pt,
        spacing,
        grid,
        snap,
        section_content_height_pt,
        Some((natural_height * 72.0 / 96.0) as f32),
    ));
    let exact = spacing
        .and_then(|s| s.line_rule.as_deref())
        .is_some_and(|rule| matches!(rule, "exact" | "exactly"));
    if !exact {
        height = height.max(natural_height);
        // Reserve the full formula before grid rounding, including when
        // explicit auto/at-least spacing suppresses the ordinary implicit floor.
        if let Some(sizing) =
            WordGridLineSizing::new(spacing, snap, grid, section_content_height_pt, font_size_pt)
                .or_else(|| {
                    WordGridLineSizing::new(
                        None,
                        snap,
                        grid,
                        section_content_height_pt,
                        font_size_pt,
                    )
                })
        {
            height = height.max(sizing.line_height_px(Some(natural_height), height));
        }
    }
    let leading = if word_grid_line_centres_text(snap, grid) {
        (height - natural_height).max(0.0) * 0.5
    } else {
        0.0
    };
    MathBlockLineBox {
        height,
        baseline: metrics.ascent + leading,
    }
}

const INLINE_GAP: f64 = 1.5;
const FRACTION_PADDING: f64 = 4.0;
const FRACTION_BAR_THICKNESS: f64 = 1.0;
const FRACTION_GAP: f64 = 3.0;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MathMetrics {
    pub width: f64,
    pub ascent: f64,
    pub descent: f64,
}

impl MathMetrics {
    fn include(&mut self, other: Self, x: f64, baseline: f64) {
        self.width = self.width.max(x + other.width);
        self.ascent = self.ascent.max(other.ascent - baseline);
        self.descent = self.descent.max(other.descent + baseline);
    }
}

pub trait MathMeasureContext {
    type Error;
    fn measure_text(&self, text: &str) -> Result<f64, Self::Error>;
}

#[derive(Debug)]
pub enum MathCommand<'a> {
    Text {
        text: Cow<'a, str>,
        x: f64,
        baseline: f64,
        width: f64,
    },
    Rule {
        start_x: f64,
        end_x: f64,
        y: f64,
        thickness: f64,
    },
}

impl MathCommand<'_> {
    fn translate(&mut self, dx: f64, dy: f64) {
        match self {
            Self::Text { x, baseline, .. } => {
                *x += dx;
                *baseline += dy;
            }
            Self::Rule {
                start_x, end_x, y, ..
            } => {
                *start_x += dx;
                *end_x += dx;
                *y += dy;
            }
        }
    }
}

#[derive(Debug)]
pub struct MathLayout<'a> {
    pub metrics: MathMetrics,
    pub commands: Vec<MathCommand<'a>>,
}

pub fn layout_math<'a, C: MathMeasureContext>(
    context: &C,
    math: &'a MathParagraph,
    font_px: f64,
) -> Result<MathLayout<'a>, C::Error> {
    let mut builder = Builder {
        context,
        font_px,
        commands: Vec::new(),
    };
    let metrics = builder.elements(&math.elements)?;
    Ok(MathLayout {
        metrics,
        commands: builder.commands,
    })
}

struct Builder<'c, 'a, C> {
    context: &'c C,
    font_px: f64,
    commands: Vec<MathCommand<'a>>,
}

impl<'a, C: MathMeasureContext> Builder<'_, 'a, C> {
    fn text(&mut self, text: Cow<'a, str>) -> Result<MathMetrics, C::Error> {
        let width = self.context.measure_text(&text)?;
        self.commands.push(MathCommand::Text {
            text,
            x: 0.0,
            baseline: 0.0,
            width,
        });
        Ok(MathMetrics {
            width,
            ascent: self.font_px * 0.8,
            descent: self.font_px * 0.2,
        })
    }

    fn shift(&mut self, range: Range<usize>, x: f64, y: f64) {
        for command in &mut self.commands[range] {
            command.translate(x, y);
        }
    }

    fn elements(&mut self, elements: &'a [MathElement]) -> Result<MathMetrics, C::Error> {
        let mut total = MathMetrics::default();
        let mut first = true;
        let mut cursor = 0.0;
        for element in elements {
            if element.tag.as_deref().is_some_and(is_property_tag) {
                continue;
            }
            if !first {
                cursor += INLINE_GAP;
            }
            first = false;
            let start = self.commands.len();
            let metrics = self.element(element)?;
            self.shift(start..self.commands.len(), cursor, 0.0);
            total.include(metrics, cursor, 0.0);
            cursor += metrics.width;
        }
        Ok(total)
    }

    fn child(
        &mut self,
        child: Option<&'a MathElement>,
    ) -> Result<(MathMetrics, Range<usize>), C::Error> {
        let start = self.commands.len();
        let metrics = child
            .map(|element| self.element(element))
            .transpose()?
            .unwrap_or_default();
        Ok((metrics, start..self.commands.len()))
    }

    fn element(&mut self, element: &'a MathElement) -> Result<MathMetrics, C::Error> {
        match element.kind {
            MathElementKind::MathParagraph | MathElementKind::Math | MathElementKind::Run => {
                self.elements(&element.children)
            }
            MathElementKind::Text => {
                self.text(Cow::Borrowed(element.text.as_deref().unwrap_or_default()))
            }
            MathElementKind::Fraction => self.fraction(element),
            MathElementKind::Superscript | MathElementKind::Subscript => self.script(element),
            MathElementKind::Nary => self.nary(element),
            MathElementKind::Radical => self.radical(element),
            MathElementKind::Function => self.function(element),
            // These remain explicit previews until the shared engine has
            // font-specific stretchy delimiter/radical metrics.
            _ => self.text(Cow::Owned(element_to_text(element))),
        }
    }

    fn fraction(&mut self, element: &'a MathElement) -> Result<MathMetrics, C::Error> {
        let (base, _) = self.child(find_child(element, MathElementKind::Base))?;
        let (numerator, num_range) = self.child(find_child(element, MathElementKind::Numerator))?;
        let (denominator, den_range) =
            self.child(find_child(element, MathElementKind::Denominator))?;
        let width = base.width.max(numerator.width).max(denominator.width) + 2.0 * FRACTION_PADDING;
        let displacement = FRACTION_GAP + FRACTION_BAR_THICKNESS + denominator.ascent;
        self.shift(num_range, (width - numerator.width) / 2.0, -displacement);
        self.shift(den_range, (width - denominator.width) / 2.0, displacement);
        self.commands.push(MathCommand::Rule {
            start_x: FRACTION_PADDING,
            end_x: width - FRACTION_PADDING,
            y: 0.0,
            thickness: FRACTION_BAR_THICKNESS,
        });
        let mut metrics = MathMetrics {
            width,
            ascent: FRACTION_BAR_THICKNESS / 2.0,
            descent: FRACTION_BAR_THICKNESS / 2.0,
        };
        metrics.include(base, 0.0, 0.0);
        if find_child(element, MathElementKind::Numerator).is_some() {
            metrics.include(numerator, 0.0, -displacement);
        }
        if find_child(element, MathElementKind::Denominator).is_some() {
            metrics.include(denominator, 0.0, displacement);
        }
        Ok(metrics)
    }

    fn script(&mut self, element: &'a MathElement) -> Result<MathMetrics, C::Error> {
        let (base, _) = self.child(find_child(element, MathElementKind::Base))?;
        let (kind, displacement) = if element.kind == MathElementKind::Superscript {
            (MathElementKind::SuperArgument, -self.font_px * 0.6)
        } else {
            (MathElementKind::SubArgument, self.font_px * 0.4)
        };
        let child = find_child(element, kind);
        let (script, range) = self.child(child)?;
        self.shift(range, base.width, displacement);
        let mut total = base;
        if child.is_some() {
            total.include(script, base.width, displacement);
        }
        Ok(total)
    }

    fn nary(&mut self, element: &'a MathElement) -> Result<MathMetrics, C::Error> {
        let symbol = nary_symbol(element).unwrap_or_else(|| "∫".to_string());
        let mut total = self.text(Cow::Owned(symbol))?;
        let symbol_width = total.width;
        for (kind, displacement) in [
            (MathElementKind::SuperArgument, -self.font_px * 0.8),
            (MathElementKind::SubArgument, self.font_px * 0.5),
        ] {
            let child = find_child(element, kind);
            let (metrics, range) = self.child(child)?;
            self.shift(range, 0.0, displacement);
            if child.is_some() {
                total.include(metrics, 0.0, displacement);
            }
        }
        let base = find_child(element, MathElementKind::Base);
        let (metrics, range) = self.child(base)?;
        let x = symbol_width + INLINE_GAP;
        self.shift(range, x, 0.0);
        if base.is_some() {
            total.include(metrics, x, 0.0);
        }
        Ok(total)
    }

    fn radical(&mut self, element: &'a MathElement) -> Result<MathMetrics, C::Error> {
        let mut text = String::from("√");
        if let Some(base) = find_child(element, MathElementKind::Base) {
            let base = element_to_text(base);
            if !base.is_empty() {
                text.push('(');
                text.push_str(&base);
                text.push(')');
            }
        }
        self.text(Cow::Owned(text))
    }

    fn function(&mut self, element: &'a MathElement) -> Result<MathMetrics, C::Error> {
        let name = find_child(element, MathElementKind::FunctionName)
            .map(element_to_text)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "f".to_string());
        let mut total = self.text(Cow::Owned(name))?;
        let base = find_child(element, MathElementKind::Base);
        if base.is_none() {
            return Ok(total);
        }
        let start = self.commands.len();
        let open = self.text(Cow::Borrowed("("))?;
        let mut x = total.width;
        self.shift(start..self.commands.len(), x, 0.0);
        total.include(open, x, 0.0);
        x += open.width;
        let (metrics, range) = self.child(base)?;
        self.shift(range, x, 0.0);
        total.include(metrics, x, 0.0);
        x += metrics.width;
        let start = self.commands.len();
        let close = self.text(Cow::Borrowed(")"))?;
        self.shift(start..self.commands.len(), x, 0.0);
        total.include(close, x, 0.0);
        Ok(total)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MathParagraphAlignment {
    #[default]
    Left,
    Center,
    Right,
}

impl MathParagraphAlignment {
    pub fn from_math_paragraph(math: &MathParagraph) -> Self {
        math.elements
            .iter()
            .find_map(Self::from_element)
            .unwrap_or_default()
    }

    fn from_element(element: &MathElement) -> Option<Self> {
        if element.tag.as_deref() == Some("oMathParaPr") {
            return element.children.iter().find_map(Self::from_property);
        }
        element.children.iter().find_map(Self::from_element)
    }

    fn from_property(element: &MathElement) -> Option<Self> {
        if element.tag.as_deref() == Some("jc") {
            return match element.attributes.get("val").map(String::as_str) {
                Some("center") | Some("centerGroup") => Some(Self::Center),
                Some("right") => Some(Self::Right),
                Some("left") => Some(Self::Left),
                _ => None,
            };
        }
        element.children.iter().find_map(Self::from_property)
    }

    pub fn x(self, column_x: f64, column_width: f64, math_width: f64) -> f64 {
        let remaining = (column_width - math_width).max(0.0);
        column_x
            + match self {
                Self::Left => 0.0,
                Self::Center => remaining / 2.0,
                Self::Right => remaining,
            }
    }
}

pub fn math_text(math: &MathParagraph) -> String {
    if math.elements.is_empty() {
        return String::new();
    }
    let mut buffer = String::new();
    elements_to_text(&math.elements, &mut buffer);
    if buffer.is_empty() {
        "math".to_string()
    } else {
        buffer
    }
}

fn elements_to_text(elements: &[MathElement], buffer: &mut String) {
    let mut first = true;
    for element in elements {
        if let Some(tag) = element.tag.as_deref() {
            if is_property_tag(tag) {
                continue;
            }
        }
        if !first {
            buffer.push(' ');
        }
        first = false;
        element_to_text_into(element, buffer);
    }
}

fn element_to_text_into(element: &MathElement, buffer: &mut String) {
    if let Some(tag) = element.tag.as_deref() {
        if is_property_tag(tag) {
            return;
        }
    }
    match element.kind {
        MathElementKind::MathParagraph | MathElementKind::Math | MathElementKind::Run => {
            for child in &element.children {
                element_to_text_into(child, buffer);
            }
        }
        MathElementKind::Text => {
            if let Some(text) = &element.text {
                buffer.push_str(text.trim());
            }
        }
        MathElementKind::Fraction => {
            let numerator = find_child(element, MathElementKind::Numerator);
            let denominator = find_child(element, MathElementKind::Denominator);
            buffer.push('(');
            if let Some(numer) = numerator {
                buffer.push_str(&element_to_text(numer));
            }
            buffer.push_str(")/(");
            if let Some(denom) = denominator {
                buffer.push_str(&element_to_text(denom));
            }
            buffer.push(')');
        }
        MathElementKind::Superscript => {
            let base = find_child(element, MathElementKind::Base);
            let super_arg = find_child(element, MathElementKind::SuperArgument);
            if let Some(base) = base {
                element_to_text_into(base, buffer);
            }
            buffer.push('^');
            if let Some(super_arg) = super_arg {
                buffer.push('(');
                element_to_text_into(super_arg, buffer);
                buffer.push(')');
            }
        }
        MathElementKind::Subscript => {
            let base = find_child(element, MathElementKind::Base);
            let sub_arg = find_child(element, MathElementKind::SubArgument);
            if let Some(base) = base {
                element_to_text_into(base, buffer);
            }
            buffer.push('_');
            if let Some(sub_arg) = sub_arg {
                buffer.push('(');
                element_to_text_into(sub_arg, buffer);
                buffer.push(')');
            }
        }
        MathElementKind::Radical => {
            buffer.push_str("√(");
            for child in &element.children {
                element_to_text_into(child, buffer);
            }
            buffer.push(')');
        }
        MathElementKind::Delimiter => {
            buffer.push('(');
            for child in &element.children {
                element_to_text_into(child, buffer);
            }
            buffer.push(')');
        }
        MathElementKind::Function => {
            let name = find_child(element, MathElementKind::FunctionName)
                .map(element_to_text)
                .unwrap_or_else(|| "f".to_string());
            buffer.push_str(&name);
            if let Some(base) = find_child(element, MathElementKind::Base) {
                buffer.push('(');
                element_to_text_into(base, buffer);
                buffer.push(')');
            }
        }
        MathElementKind::Nary => {
            let symbol = nary_symbol(element).unwrap_or_else(|| "∫".to_string());
            buffer.push_str(&symbol);
            if let Some(sub) = find_child(element, MathElementKind::SubArgument) {
                buffer.push('_');
                buffer.push('(');
                element_to_text_into(sub, buffer);
                buffer.push(')');
            }
            if let Some(sup) = find_child(element, MathElementKind::SuperArgument) {
                buffer.push('^');
                buffer.push('(');
                element_to_text_into(sup, buffer);
                buffer.push(')');
            }
            if let Some(base) = find_child(element, MathElementKind::Base) {
                buffer.push(' ');
                element_to_text_into(base, buffer);
            }
        }
        _ => {
            for child in &element.children {
                element_to_text_into(child, buffer);
            }
        }
    }
}

fn element_to_text(element: &MathElement) -> String {
    let mut buffer = String::new();
    element_to_text_into(element, &mut buffer);
    buffer
}

fn find_child(element: &MathElement, kind: MathElementKind) -> Option<&MathElement> {
    element.children.iter().find(|child| child.kind == kind)
}

fn is_property_tag(tag: &str) -> bool {
    tag.ends_with("Pr") || matches!(tag, "ctrlPr" | "rPr")
}

fn nary_symbol(element: &MathElement) -> Option<String> {
    if let Some(value) = element.attributes.get("chr") {
        if !value.is_empty() {
            return Some(value.clone());
        }
    }

    if let Some(pr) = element
        .children
        .iter()
        .find(|child| child.tag.as_deref() == Some("naryPr"))
    {
        if let Some(chr) = pr
            .children
            .iter()
            .find(|child| child.tag.as_deref() == Some("chr"))
        {
            if let Some(value) = chr.attributes.get("val") {
                if !value.is_empty() {
                    return Some(value.clone());
                }
            }
            if let Some(text) = chr.text.as_ref() {
                if !text.is_empty() {
                    return Some(text.clone());
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Measure {
        calls: RefCell<Vec<String>>,
    }
    impl MathMeasureContext for Measure {
        type Error = &'static str;
        fn measure_text(&self, text: &str) -> Result<f64, Self::Error> {
            self.calls.borrow_mut().push(text.to_string());
            if text == "unavailable" {
                return Err("font unavailable");
            }
            Ok(text.chars().count() as f64 * 8.0)
        }
    }
    fn parse(body: &str) -> MathParagraph {
        jade_docx_reader::parser::math::parse_math_paragraph(format!(
            "<m:oMath xmlns:m=\"http://schemas.openxmlformats.org/officeDocument/2006/math\">{body}</m:oMath>"
        )).unwrap()
    }

    #[test]
    fn property_nodes_do_not_consume_width_or_measurements() {
        let math = parse("<m:r><m:rPr><m:sty m:val=\"i\"/></m:rPr><m:t>x</m:t></m:r>");
        let measure = Measure::default();
        let layout = layout_math(&measure, &math, 16.0).unwrap();
        assert_eq!(&*measure.calls.borrow(), &["x"]);
        assert_eq!(layout.metrics.width, 8.0);
        assert!(matches!(
            &layout.commands[0],
            MathCommand::Text {
                text: Cow::Borrowed("x"),
                ..
            }
        ));
    }

    #[test]
    fn math_block_box_reuses_logical_character_and_mirrored_indents() {
        use jade_docx_reader::{Indent, ParagraphProperties};
        let mut properties = ParagraphProperties {
            indent: Some(Indent {
                left: Some(1.0),
                right: Some(2.0),
                start: Some(30.0),
                end: Some(15.0),
                start_chars: Some(200),
                end_chars: Some(100),
                first_line_chars: Some(50),
                ..Indent::default()
            }),
            mirror_indents: Some(true),
            ..ParagraphProperties::default()
        };
        assert_eq!(
            math_block_content_box(Some(&properties), 100.0, 300.0, 10.5, false),
            (135.0, 251.0)
        );
        assert_eq!(
            math_block_content_box(Some(&properties), 100.0, 300.0, 10.5, true),
            (121.0, 251.0)
        );
        properties.bidi = Some(true);
        assert_eq!(
            math_block_content_box(Some(&properties), 100.0, 300.0, 10.5, false),
            (121.0, 251.0)
        );
        assert_eq!(
            math_block_content_box(Some(&properties), 100.0, 300.0, 10.5, true),
            (135.0, 251.0)
        );
        assert_eq!(
            math_block_content_box(None, 100.0, 300.0, 10.5, false),
            (100.0, 300.0)
        );
    }

    #[test]
    fn every_paint_command_fits_inside_reported_math_bounds() {
        for body in [
            "<m:f><m:num><m:r><m:t>wide</m:t></m:r></m:num><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f>",
            "<m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup>",
            "<m:sSub><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sub><m:r><m:t>2</m:t></m:r></m:sub></m:sSub>",
            "<m:nary><m:naryPr><m:chr m:val=\"∑\"/></m:naryPr><m:sub><m:r><m:t>lower</m:t></m:r></m:sub><m:sup><m:r><m:t>upper</m:t></m:r></m:sup><m:e><m:r><m:t>x</m:t></m:r></m:e></m:nary>",
        ] {
            let math = parse(body);
            let measure = Measure::default();
            let layout = layout_math(&measure, &math, 16.0).unwrap();
            let mut texts = 0;
            for command in &layout.commands {
                match command {
                    MathCommand::Text {x, baseline, width, ..} => {
                        texts += 1;
                        assert!(*x >= 0.0 && x + width <= layout.metrics.width + 1e-9);
                        assert!(baseline - 12.8 >= -layout.metrics.ascent - 1e-9);
                        assert!(baseline + 3.2 <= layout.metrics.descent + 1e-9);
                    }
                    MathCommand::Rule {start_x, end_x, y, thickness} => {
                        assert!(*start_x >= 0.0 && *end_x <= layout.metrics.width);
                        assert!(y - thickness / 2.0 >= -layout.metrics.ascent);
                        assert!(y + thickness / 2.0 <= layout.metrics.descent);
                    }
                }
            }
            assert_eq!(measure.calls.borrow().len(), texts, "each emitted text is measured only once: {body}");
        }
    }

    #[test]
    fn failed_measurement_propagates_without_painting_a_partial_plan() {
        let math = parse("<m:r><m:t>unavailable</m:t></m:r><m:r><m:t>later</m:t></m:r>");
        let measure = Measure::default();
        let result = layout_math(&measure, &math, 16.0);
        assert_eq!(result.unwrap_err(), "font unavailable");
        assert_eq!(&*measure.calls.borrow(), &["unavailable"]);
    }

    #[test]
    fn radical_fallback_keeps_existing_base_only_preview() {
        let math = parse("<m:rad><m:deg><m:r><m:t>3</m:t></m:r></m:deg><m:e><m:r><m:t>x</m:t></m:r></m:e></m:rad>");
        let measure = Measure::default();
        layout_math(&measure, &math, 16.0).unwrap();
        assert_eq!(&*measure.calls.borrow(), &["√(x)"]);
    }
}
