//! Host-supplied OpenType MATH data in font design units.
//!
//! Parsing borrows the original bytes. No font discovery or independent wrapping
//! or registration policy lives here. Static outlines are resolution independent;
//! device corrections and variable-font instances require a separate host policy.
use ttf_parser::{gsub, math, opentype_layout::Feature, Face, GlyphId, OutlineBuilder, Tag};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathFontError {
    InvalidFont,
    MissingMathTable,
    UnsupportedMathVersion,
    IncompleteConstants,
    VariableFont,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MathFontConstants {
    pub script_percent: i16,
    pub script_script_percent: i16,
    pub delimited_min_height: u16,
    pub display_operator_min_height: u16,
    pub leading: i16,
    pub axis_height: i16,
    pub superscript_shift_up: i16,
    pub subscript_shift_down: i16,
    pub subscript_top_max: i16,
    pub superscript_bottom_min: i16,
    pub sub_superscript_gap_min: i16,
    pub superscript_bottom_max_with_subscript: i16,
    pub superscript_baseline_drop_max: i16,
    pub subscript_baseline_drop_min: i16,
    pub upper_limit_gap_min: i16,
    pub upper_limit_baseline_rise_min: i16,
    pub lower_limit_gap_min: i16,
    pub lower_limit_baseline_drop_min: i16,
    pub space_after_script: i16,
    pub accent_base_height: i16,
    pub flattened_accent_base_height: i16,
    pub superscript_shift_up_cramped: i16,
    pub stack_top_shift_up: i16,
    pub stack_top_display_style_shift_up: i16,
    pub stack_bottom_shift_down: i16,
    pub stack_bottom_display_style_shift_down: i16,
    pub stack_gap_min: i16,
    pub stack_display_style_gap_min: i16,
    pub stretch_stack_top_shift_up: i16,
    pub stretch_stack_bottom_shift_down: i16,
    pub stretch_stack_gap_above_min: i16,
    pub stretch_stack_gap_below_min: i16,
    pub fraction_numerator_shift_up: i16,
    pub fraction_numerator_display_style_shift_up: i16,
    pub fraction_denominator_shift_down: i16,
    pub fraction_denominator_display_style_shift_down: i16,
    pub fraction_numerator_gap_min: i16,
    pub fraction_num_display_style_gap_min: i16,
    pub fraction_rule_thickness: i16,
    pub fraction_denominator_gap_min: i16,
    pub fraction_denom_display_style_gap_min: i16,
    pub skewed_fraction_horizontal_gap: i16,
    pub skewed_fraction_vertical_gap: i16,
    pub overbar_vertical_gap: i16,
    pub overbar_rule_thickness: i16,
    pub overbar_extra_ascender: i16,
    pub underbar_vertical_gap: i16,
    pub underbar_rule_thickness: i16,
    pub underbar_extra_descender: i16,
    pub radical_vertical_gap: i16,
    pub radical_display_style_vertical_gap: i16,
    pub radical_rule_thickness: i16,
    pub radical_extra_ascender: i16,
    pub radical_kern_before_degree: i16,
    pub radical_kern_after_degree: i16,
    pub radical_degree_bottom_raise_percent: i16,
    pub min_connector_overlap: u16,
}
impl MathFontConstants {
    /// Invalid percentages are reported rather than turned into guessed metrics.
    pub fn script_scale(self, level: u32) -> Option<f64> {
        let percent = match level {
            0 => return Some(1.0),
            1 => self.script_percent,
            _ => self.script_script_percent,
        };
        (1..=100)
            .contains(&percent)
            .then_some(f64::from(percent) / 100.0)
    }
}

/// Font coordinates: y grows upwards from the baseline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MathGlyphBounds {
    pub x_min: f64,
    pub y_min: f64,
    pub x_max: f64,
    pub y_max: f64,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MathFontGlyph {
    pub glyph_id: u16,
    pub advance_px: f64,
    pub ink_px: MathGlyphBounds,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MathGlyphVariant {
    pub glyph_id: u16,
    /// Extension extent, not the horizontal text advance or the glyph ink box.
    pub extension_units: u16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathStretchAxis {
    Vertical,
    Horizontal,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MathGlyphPart {
    pub glyph_id: u16,
    pub start_connector: u16,
    pub end_connector: u16,
    pub full_advance: u16,
    pub extender: bool,
}
#[derive(Debug, Clone, Copy)]
pub struct MathGlyphAssembly<'a> {
    source: math::GlyphAssembly<'a>,
    pub min_connector_overlap: u16,
    pub italic_correction: i16,
}
impl MathGlyphAssembly<'_> {
    /// Vertical parts are bottom-to-top; horizontal parts are left-to-right.
    /// This is a recipe, not a completed or verified stretch to the target.
    pub fn parts(&self) -> impl Iterator<Item = MathGlyphPart> + '_ {
        self.source.parts.into_iter().map(|p| MathGlyphPart {
            glyph_id: p.glyph_id.0,
            start_connector: p.start_connector_length,
            end_connector: p.end_connector_length,
            full_advance: p.full_advance,
            extender: p.part_flags.extender(),
        })
    }
}
/// What a character grown to a size is drawn with: one glyph, or parts at
/// offsets (font units, from the bottom or the left) along the axis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MathStretchChoice {
    Variant(u16),
    Assembly(Vec<(u16, u32)>),
}

#[derive(Debug, Clone, Copy)]
pub enum MathStretchPlan<'a> {
    Glyph(MathGlyphVariant),
    Assembly(MathGlyphAssembly<'a>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MathOutlineCommand {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}
#[derive(Debug, Clone, PartialEq)]
pub struct MathGlyphOutline {
    /// Unscaled design units; invert y when consumed by a Canvas-like painter.
    pub commands: Vec<MathOutlineCommand>,
    pub bounds: MathGlyphBounds,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathOutlineError {
    CommandBudgetExceeded,
}

/// A borrowed face; the caller retains font bytes and chooses the actual face.
pub struct OpenTypeMathFont<'a> {
    face: Face<'a>,
    table: math::Table<'a>,
    constants: MathFontConstants,
    // Borrowed table view, selected once per parsed face; no allocation.
    script_style: Option<Feature<'a>>,
}
impl<'a> OpenTypeMathFont<'a> {
    pub fn parse(bytes: &'a [u8], face_index: u32) -> Result<Self, MathFontError> {
        let face = Face::parse(bytes, face_index).map_err(|_| MathFontError::InvalidFont)?;
        if face.raw_face().table(Tag::from_bytes(b"fvar")).is_some() {
            return Err(MathFontError::VariableFont);
        }
        let raw = face
            .raw_face()
            .table(Tag::from_bytes(b"MATH"))
            .ok_or(MathFontError::MissingMathTable)?;
        if raw.get(..4) != Some(&[0, 1, 0, 0]) {
            return Err(MathFontError::UnsupportedMathVersion);
        }
        let offset = raw
            .get(4..6)
            .map(|v| usize::from(u16::from_be_bytes([v[0], v[1]])))
            .filter(|v| *v >= 10)
            .ok_or(MathFontError::IncompleteConstants)?;
        // Through RadicalDegreeBottomRaisePercent, including all value records.
        // The upstream lazy reader otherwise returns zero for truncated fields.
        raw.get(offset..offset + 214)
            .ok_or(MathFontError::IncompleteConstants)?;
        let table = face.tables().math.ok_or(MathFontError::MissingMathTable)?;
        let c = table.constants.ok_or(MathFontError::IncompleteConstants)?;
        let constants = MathFontConstants {
            script_percent: c.script_percent_scale_down(),
            script_script_percent: c.script_script_percent_scale_down(),
            delimited_min_height: c.delimited_sub_formula_min_height(),
            display_operator_min_height: c.display_operator_min_height(),
            leading: c.math_leading().value,
            axis_height: c.axis_height().value,
            superscript_shift_up: c.superscript_shift_up().value,
            subscript_shift_down: c.subscript_shift_down().value,
            subscript_top_max: c.subscript_top_max().value,
            superscript_bottom_min: c.superscript_bottom_min().value,
            sub_superscript_gap_min: c.sub_superscript_gap_min().value,
            superscript_bottom_max_with_subscript: c.superscript_bottom_max_with_subscript().value,
            superscript_baseline_drop_max: c.superscript_baseline_drop_max().value,
            subscript_baseline_drop_min: c.subscript_baseline_drop_min().value,
            upper_limit_gap_min: c.upper_limit_gap_min().value,
            upper_limit_baseline_rise_min: c.upper_limit_baseline_rise_min().value,
            lower_limit_gap_min: c.lower_limit_gap_min().value,
            lower_limit_baseline_drop_min: c.lower_limit_baseline_drop_min().value,
            space_after_script: c.space_after_script().value,
            accent_base_height: c.accent_base_height().value,
            flattened_accent_base_height: c.flattened_accent_base_height().value,
            superscript_shift_up_cramped: c.superscript_shift_up_cramped().value,
            stack_top_shift_up: c.stack_top_shift_up().value,
            stack_top_display_style_shift_up: c.stack_top_display_style_shift_up().value,
            stack_bottom_shift_down: c.stack_bottom_shift_down().value,
            stack_bottom_display_style_shift_down: c.stack_bottom_display_style_shift_down().value,
            stack_gap_min: c.stack_gap_min().value,
            stack_display_style_gap_min: c.stack_display_style_gap_min().value,
            stretch_stack_top_shift_up: c.stretch_stack_top_shift_up().value,
            stretch_stack_bottom_shift_down: c.stretch_stack_bottom_shift_down().value,
            stretch_stack_gap_above_min: c.stretch_stack_gap_above_min().value,
            stretch_stack_gap_below_min: c.stretch_stack_gap_below_min().value,
            fraction_numerator_shift_up: c.fraction_numerator_shift_up().value,
            fraction_numerator_display_style_shift_up: c
                .fraction_numerator_display_style_shift_up()
                .value,
            fraction_denominator_shift_down: c.fraction_denominator_shift_down().value,
            fraction_denominator_display_style_shift_down: c
                .fraction_denominator_display_style_shift_down()
                .value,
            fraction_numerator_gap_min: c.fraction_numerator_gap_min().value,
            fraction_num_display_style_gap_min: c.fraction_num_display_style_gap_min().value,
            fraction_rule_thickness: c.fraction_rule_thickness().value,
            fraction_denominator_gap_min: c.fraction_denominator_gap_min().value,
            fraction_denom_display_style_gap_min: c.fraction_denom_display_style_gap_min().value,
            skewed_fraction_horizontal_gap: c.skewed_fraction_horizontal_gap().value,
            skewed_fraction_vertical_gap: c.skewed_fraction_vertical_gap().value,
            overbar_vertical_gap: c.overbar_vertical_gap().value,
            overbar_rule_thickness: c.overbar_rule_thickness().value,
            overbar_extra_ascender: c.overbar_extra_ascender().value,
            underbar_vertical_gap: c.underbar_vertical_gap().value,
            underbar_rule_thickness: c.underbar_rule_thickness().value,
            underbar_extra_descender: c.underbar_extra_descender().value,
            radical_vertical_gap: c.radical_vertical_gap().value,
            radical_display_style_vertical_gap: c.radical_display_style_vertical_gap().value,
            radical_rule_thickness: c.radical_rule_thickness().value,
            radical_extra_ascender: c.radical_extra_ascender().value,
            radical_kern_before_degree: c.radical_kern_before_degree().value,
            radical_kern_after_degree: c.radical_kern_after_degree().value,
            radical_degree_bottom_raise_percent: c.radical_degree_bottom_raise_percent(),
            min_connector_overlap: table.variants.map_or(0, |v| v.min_connector_overlap),
        };
        let script_style = face.tables().gsub.and_then(|gsub| {
            for script_tag in [b"math", b"DFLT"] {
                if let Some(language) = gsub
                    .scripts
                    .find(Tag::from_bytes(script_tag))
                    .and_then(|script| script.default_language)
                {
                    for index in language
                        .required_feature
                        .into_iter()
                        .chain(language.feature_indices)
                    {
                        if let Some(feature) = gsub
                            .features
                            .get(index)
                            .filter(|feature| feature.tag == Tag::from_bytes(b"ssty"))
                        {
                            return Some(feature);
                        }
                    }
                }
            }
            // A sole global declaration is unambiguous even in fonts without
            // a math/default language system. Never pick a random duplicate.
            let mut features = gsub
                .features
                .into_iter()
                .filter(|feature| feature.tag == Tag::from_bytes(b"ssty"));
            let feature = features.next()?;
            features.next().is_none().then_some(feature)
        });
        Ok(Self {
            face,
            table,
            constants,
            script_style,
        })
    }
    pub fn units_per_em(&self) -> u16 {
        self.face.units_per_em()
    }
    pub fn constants(&self) -> MathFontConstants {
        self.constants
    }
    /// The chosen face's typographic box, with its line gap above the baseline.
    /// Static faces without OS/2 use their ordinary horizontal metrics.
    pub fn line_metrics(&self, font_size_px: f64) -> Option<super::MathMetrics> {
        let scale = self.scale(font_size_px)?;
        let ascent = self
            .face
            .typographic_ascender()
            .unwrap_or_else(|| self.face.ascender());
        let descent = self
            .face
            .typographic_descender()
            .unwrap_or_else(|| self.face.descender());
        let gap = self
            .face
            .typographic_line_gap()
            .unwrap_or_else(|| self.face.line_gap());
        Some(super::MathMetrics {
            width: 0.0,
            ascent: ((f64::from(ascent) + f64::from(gap)) * scale).max(0.0),
            descent: (-f64::from(descent) * scale).max(0.0),
        })
    }
    pub fn glyph_id(&self, character: char) -> Option<u16> {
        self.face.glyph_index(character).map(|g| g.0)
    }
    /// OpenType ssty selects optical forms by math nesting, not by font size.
    /// The layout engine still scales and positions the chosen glyph once.
    /// Only the specified single/alternate lookups are handled; unsupported or
    /// invalid lookup data conservatively retain the original glyph.
    pub fn script_glyph_id(&self, original: u16, level: u32) -> u16 {
        if level == 0 {
            return original;
        }
        let Some(feature) = self.script_style else {
            return original;
        };
        let Some(table) = self.face.tables().gsub else {
            return original;
        };
        let mut glyph = GlyphId(original);
        for index in feature.lookup_indices {
            let Some(lookup) = table.lookups.get(index) else {
                return original;
            };
            // Contextual substitutions need a glyph run, not a single scalar.
            if lookup.flags.0 != 0 {
                return original;
            }
            for subtable_index in 0..lookup.subtables.len() {
                let Some(subtable) = lookup
                    .subtables
                    .get::<gsub::SubstitutionSubtable>(subtable_index)
                else {
                    return original;
                };
                let next = match subtable {
                    gsub::SubstitutionSubtable::Single(single) => {
                        let Some(index) = single.coverage().get(glyph) else {
                            continue;
                        };
                        match single {
                            gsub::SingleSubstitution::Format1 { delta, .. } => {
                                Some(GlyphId(glyph.0.wrapping_add(delta as u16)))
                            }
                            gsub::SingleSubstitution::Format2 { substitutes, .. } => {
                                let Some(next) = substitutes.get(index) else {
                                    return original;
                                };
                                Some(next)
                            }
                        }
                    }
                    gsub::SubstitutionSubtable::Alternate(alternate) => {
                        let Some(index) = alternate.coverage.get(glyph) else {
                            continue;
                        };
                        let Some(set) = alternate.alternate_sets.get(index) else {
                            return original;
                        };
                        set.alternates.get(level.min(2) as u16 - 1)
                    }
                    _ => return original,
                };
                if let Some(next) = next {
                    if next.0 >= self.face.number_of_glyphs() {
                        return original;
                    }
                    glyph = next;
                }
                break;
            }
        }
        glyph.0
    }

    pub fn glyph_advance_units(&self, glyph: u16) -> Option<u16> {
        if glyph >= self.face.number_of_glyphs() {
            return None;
        }
        self.face.glyph_hor_advance(GlyphId(glyph))
    }
    pub fn scale(&self, font_size_px: f64) -> Option<f64> {
        (font_size_px.is_finite() && font_size_px > 0.0)
            .then_some(font_size_px / f64::from(self.units_per_em()))
    }
    pub fn glyph_geometry(&self, glyph: u16, font_size_px: f64) -> Option<MathFontGlyph> {
        let scale = self.scale(font_size_px)?;
        let glyph_id = GlyphId(glyph);
        let bounds = self.face.glyph_bounding_box(glyph_id)?;
        Some(MathFontGlyph {
            glyph_id: glyph,
            advance_px: f64::from(self.face.glyph_hor_advance(glyph_id)?) * scale,
            ink_px: MathGlyphBounds {
                x_min: f64::from(bounds.x_min) * scale,
                y_min: f64::from(bounds.y_min) * scale,
                x_max: f64::from(bounds.x_max) * scale,
                y_max: f64::from(bounds.y_max) * scale,
            },
        })
    }
    pub fn italic_correction(&self, glyph: u16) -> i16 {
        self.table
            .glyph_info
            .and_then(|g| g.italic_corrections)
            .and_then(|v| v.get(GlyphId(glyph)))
            .map_or(0, |v| v.value)
    }
    /// Where an accent sits over this glyph, in font units from its origin.
    pub fn top_accent_attachment(&self, glyph: u16) -> Option<i16> {
        self.table
            .glyph_info
            .and_then(|g| g.top_accent_attachments)
            .and_then(|v| v.get(GlyphId(glyph)))
            .map(|v| v.value)
    }
    pub fn is_extended_shape(&self, glyph: u16) -> bool {
        self.table
            .glyph_info
            .and_then(|g| g.extended_shapes)
            .is_some_and(|c| c.get(GlyphId(glyph)).is_some())
    }
    /// The smallest variant at least `target_units` long, else the assembly
    /// grown to that length (parts repeated and their overlaps shared out),
    /// else the largest variant there is.
    pub fn stretch_choice(
        &self,
        glyph: u16,
        axis: MathStretchAxis,
        target_units: u32,
    ) -> Option<MathStretchChoice> {
        let variants = self.table.variants?;
        let construction = match axis {
            MathStretchAxis::Vertical => variants.vertical_constructions,
            MathStretchAxis::Horizontal => variants.horizontal_constructions,
        }
        .get(GlyphId(glyph))?;
        let glyphs = self.face.number_of_glyphs();
        let mut largest = None;
        for v in construction.variants {
            if v.variant_glyph.0 >= glyphs {
                continue;
            }
            if u32::from(v.advance_measurement) >= target_units {
                return Some(MathStretchChoice::Variant(v.variant_glyph.0));
            }
            largest = Some(v.variant_glyph.0);
        }
        if let Some(assembly) = construction.assembly {
            let parts: Vec<_> = assembly.parts.into_iter().collect();
            if parts.iter().any(|p| p.glyph_id.0 >= glyphs) || parts.is_empty() {
                return largest.map(MathStretchChoice::Variant);
            }
            let min_overlap = u32::from(variants.min_connector_overlap);
            let mut sequence = Vec::new();
            for repeats in 0..=64u32 {
                sequence.clear();
                for p in &parts {
                    let count = if p.part_flags.extender() { repeats } else { 1 };
                    for _ in 0..count {
                        sequence.push(*p);
                    }
                }
                if sequence.is_empty() {
                    continue;
                }
                let sum: u32 = sequence.iter().map(|p| u32::from(p.full_advance)).sum();
                let joints = sequence.len() as u32 - 1;
                if sum.saturating_sub(min_overlap * joints) >= target_units {
                    break;
                }
            }
            let sum: u32 = sequence.iter().map(|p| u32::from(p.full_advance)).sum();
            let maxima: Vec<u32> = sequence
                .windows(2)
                .map(|w| {
                    u32::from(w[0].end_connector_length.min(w[1].start_connector_length))
                        .max(min_overlap)
                })
                .collect();
            let floor = min_overlap * maxima.len() as u32;
            let ceiling: u32 = maxima.iter().sum();
            let wanted = sum
                .saturating_sub(target_units)
                .clamp(floor, ceiling.max(floor));
            let room = ceiling.saturating_sub(floor).max(1);
            let mut placed = Vec::with_capacity(sequence.len());
            let mut offset = 0u32;
            for (i, part) in sequence.iter().enumerate() {
                placed.push((part.glyph_id.0, offset));
                if let Some(max) = maxima.get(i) {
                    let share =
                        u64::from(wanted - floor) * u64::from(max - min_overlap) / u64::from(room);
                    let overlap = min_overlap + share as u32;
                    offset += u32::from(part.full_advance).saturating_sub(overlap);
                }
            }
            return Some(MathStretchChoice::Assembly(placed));
        }
        largest.map(MathStretchChoice::Variant)
    }

    /// Smallest adequate prepared variant. A larger target requires an assembly;
    /// the last prepared glyph is never represented as satisfying that target.
    pub fn stretch_plan(
        &self,
        glyph: u16,
        axis: MathStretchAxis,
        target_units: u32,
    ) -> Option<MathStretchPlan<'a>> {
        let variants = self.table.variants?;
        let construction = match axis {
            MathStretchAxis::Vertical => variants.vertical_constructions,
            MathStretchAxis::Horizontal => variants.horizontal_constructions,
        }
        .get(GlyphId(glyph))?;
        if let Some(v) = construction
            .variants
            .into_iter()
            .filter(|v| {
                u32::from(v.advance_measurement) >= target_units
                    && v.variant_glyph.0 < self.face.number_of_glyphs()
            })
            .min_by_key(|v| v.advance_measurement)
        {
            return Some(MathStretchPlan::Glyph(MathGlyphVariant {
                glyph_id: v.variant_glyph.0,
                extension_units: v.advance_measurement,
            }));
        }
        let source = construction.assembly?;
        if source
            .parts
            .into_iter()
            .any(|p| p.glyph_id.0 >= self.face.number_of_glyphs())
        {
            return None;
        }
        Some(MathStretchPlan::Assembly(MathGlyphAssembly {
            min_connector_overlap: variants.min_connector_overlap,
            italic_correction: source.italics_correction.value,
            source,
        }))
    }
    /// Decode an unencoded display variant for a backend to paint, once, from
    /// exactly the glyph used for metrics. Never return a partially decoded path.
    pub fn glyph_outline(
        &self,
        glyph: u16,
        max_commands: usize,
    ) -> Result<Option<MathGlyphOutline>, MathOutlineError> {
        let mut builder = BoundedOutline {
            commands: Vec::new(),
            limit: max_commands,
            exceeded: false,
        };
        let bounds = self.face.outline_glyph(GlyphId(glyph), &mut builder);
        if builder.exceeded {
            return Err(MathOutlineError::CommandBudgetExceeded);
        }
        Ok(bounds.map(|r| MathGlyphOutline {
            commands: builder.commands,
            bounds: MathGlyphBounds {
                x_min: f64::from(r.x_min),
                y_min: f64::from(r.y_min),
                x_max: f64::from(r.x_max),
                y_max: f64::from(r.y_max),
            },
        }))
    }
}
struct BoundedOutline {
    commands: Vec<MathOutlineCommand>,
    limit: usize,
    exceeded: bool,
}
impl BoundedOutline {
    fn push(&mut self, command: MathOutlineCommand) {
        if self.commands.len() < self.limit {
            self.commands.push(command);
        } else {
            self.exceeded = true;
        }
    }
}
impl OutlineBuilder for BoundedOutline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.push(MathOutlineCommand::Move(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.push(MathOutlineCommand::Line(x, y));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.push(MathOutlineCommand::Quad(cx, cy, x, y));
    }
    fn curve_to(&mut self, cx1: f32, cy1: f32, cx2: f32, cy2: f32, x: f32, y: f32) {
        self.push(MathOutlineCommand::Cubic(cx1, cy1, cx2, cy2, x, y));
    }
    fn close(&mut self) {
        self.push(MathOutlineCommand::Close);
    }
}
