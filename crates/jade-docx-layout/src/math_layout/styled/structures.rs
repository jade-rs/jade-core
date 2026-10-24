//! The OMML structures: fractions, radicals, scripts and prescripts, n-ary
//! operators, delimiters, functions, accents, bars, group characters, limits,
//! boxes, border boxes, phantoms, equation arrays and matrices.
//!
//! Each lays its arguments out at the origin, works out where they go from
//! the font's MATH constants (Cambria Math's when no face is bound), moves
//! them there, and draws its own marks last so they land where the
//! arguments did.
use super::{
    child, children_of, collect_text, has_content, is_skipped, on, on_unless_off, property,
    property_element, shapes, AtomClass, Builder, Mark, MathConstants, MathPathCommand, MathRule,
    MathShape, StyledMathMeasure,
};
use crate::math_layout::{nary_symbol, MathMetrics};
use jade_docx_reader::document::{MathElement, MathElementKind};

/// A stretched character ready to draw: its outline relative to the glyph's
/// origin on the structure's baseline, and its new size.
pub(crate) struct Stretched {
    pub(crate) width: f64,
    pub(crate) ascent: f64,
    pub(crate) descent: f64,
    /// Path and whether it is filled (else stroked at `stroke`).
    pub(crate) paths: Vec<(Vec<MathPathCommand>, bool)>,
    pub(crate) stroke: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Axis {
    Vertical,
}

const INTEGRALS: &str = "∫∬∭∮∯∰∱∲∳⨌";

impl<'a, C: StyledMathMeasure> Builder<'_, 'a, C> {
    /// Lays out a piece at the origin and returns where its items went.
    fn piece<F>(&mut self, f: F) -> Result<(Mark, Mark, MathMetrics), C::Error>
    where
        F: FnOnce(&mut Self) -> Result<MathMetrics, C::Error>,
    {
        let start = self.mark();
        let metrics = f(self)?;
        Ok((start, self.mark(), metrics))
    }

    /// A glyph for `text` outside any source element (a fraction's slash, a
    /// function's parentheses), spaced as nothing.
    fn bare_text(&mut self, text: &str) -> Result<MathMetrics, C::Error> {
        let saved = (self.prev.take(), self.first, self.normal_text);
        self.normal_text = true;
        let m = self.text(text);
        self.prev = saved.0;
        self.first = saved.1;
        self.normal_text = saved.2;
        m
    }

    /// Whether glyphs of the current style come from a bound math face.
    pub(crate) fn font_bound(&self) -> bool {
        #[cfg(feature = "opentype-math")]
        {
            self.context.font_parameters(self.sized_style()).is_some()
        }
        #[cfg(not(feature = "opentype-math"))]
        {
            false
        }
    }

    fn rule_thickness(&self, c: &MathConstants) -> f64 {
        c.fraction_rule_thickness.max(0.5)
    }

    /// Plans a stretched form of glyph `index`, `target` long along `axis`.
    /// Vertical forms are centred on `center_y`. `None` keeps the glyph.
    pub(crate) fn plan_stretch(
        &self,
        index: usize,
        axis: Axis,
        target: f64,
        center_y: f64,
    ) -> Result<Option<Stretched>, C::Error> {
        let g = &self.plan.glyphs[index];
        let text = &self.plan.text[g.text_range.clone()];
        let Some(ch) = text.chars().next() else {
            return Ok(None);
        };
        let Axis::Vertical = axis;
        let natural = g.ascent + g.descent;
        if target <= natural * 1.02 {
            return Ok(None);
        }
        let c = self.constants();
        #[cfg(feature = "opentype-math")]
        {
            let font_axis = crate::math_layout::font::MathStretchAxis::Vertical;
            if let Some(st) = self
                .context
                .stretch_glyph(text, g.style, font_axis, target)?
            {
                let dy = center_y - (st.descent - st.ascent) / 2.0;
                let mut paths = Vec::new();
                for part in &st.parts {
                    paths.push((
                        super::font::outline_commands(&part.glyph, part.dx, dy + part.dy),
                        true,
                    ));
                }
                return Ok(Some(Stretched {
                    width: st.advance,
                    ascent: st.ascent - dy,
                    descent: st.descent + dy,
                    paths,
                    stroke: 0.0,
                }));
            }
        }
        let thickness = self.rule_thickness(&c);
        if !shapes::has_vertical(ch) {
            return Ok(None);
        }
        let grow = (target / natural.max(1.0) - 1.0).max(0.0);
        let width = (g.width * (1.0 + 0.12 * grow))
            .min(g.width * 2.2)
            .max(thickness * 3.0);
        let top = center_y - target / 2.0;
        let Some((path, fill)) = shapes::vertical(ch, 0.0, top, width, target, thickness) else {
            return Ok(None);
        };
        Ok(Some(Stretched {
            width,
            ascent: -top,
            descent: top + target,
            paths: vec![(path, fill)],
            stroke: thickness,
        }))
    }

    /// Draws a planned stretch for glyph `index` where the glyph now is.
    pub(crate) fn emit_stretch(&mut self, index: usize, st: Stretched) {
        let (x, baseline, color) = {
            let g = &mut self.plan.glyphs[index];
            g.visible = false;
            g.width = st.width;
            g.ascent = st.ascent;
            g.descent = st.descent;
            (g.x, g.baseline, g.style.color)
        };
        for (mut commands, fill) in st.paths {
            for c in &mut commands {
                c.translate(x, baseline);
            }
            self.plan.shapes.push(MathShape {
                commands,
                stroke: (!fill).then_some(st.stroke),
                color,
            });
        }
    }

    /// Draws a horizontal stretchy character (`ch`) `width` wide with its top
    /// at `top`, from the face or as a path. Returns its height.
    fn horizontal_mark(
        &mut self,
        ch: char,
        x: f64,
        top: f64,
        width: f64,
        height: f64,
    ) -> Result<f64, C::Error> {
        let c = self.constants();
        let thickness = self.rule_thickness(&c);
        let color = self.style.color;
        #[cfg(feature = "opentype-math")]
        {
            let mut text = [0u8; 4];
            let text = ch.encode_utf8(&mut text);
            if let Some(st) = self.context.stretch_glyph(
                text,
                self.sized_style(),
                crate::math_layout::font::MathStretchAxis::Horizontal,
                width,
            )? {
                // Centre the stretched form over the span, its ink top at `top`.
                let dx = x + (width - st.advance) / 2.0;
                let dy = top + st.ascent;
                for part in &st.parts {
                    let commands =
                        super::font::outline_commands(&part.glyph, dx + part.dx, dy + part.dy);
                    self.plan.shapes.push(MathShape {
                        commands,
                        stroke: None,
                        color,
                    });
                }
                return Ok(st.ascent + st.descent);
            }
        }
        if let Some((commands, fill)) = shapes::horizontal(ch, x, top, width, height, thickness) {
            self.plan.shapes.push(MathShape {
                commands,
                stroke: (!fill).then_some(thickness),
                color,
            });
            return Ok(height);
        }
        Ok(0.0)
    }

    pub(crate) fn fraction(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let pr = property_element(e, "fPr");
        let kind = property(pr, "type").unwrap_or("bar");
        let num = child(e, Numerator);
        let den = child(e, Denominator);
        let display = self.display && !self.options.settings.small_fractions;
        let level_up = u32::from(!display);
        let c = self.constants();
        match kind {
            "lin" => {
                let (_, _, n) = self.piece(|b| b.slot(num, 0, false, false))?;
                let (s0, _, slash) = self.piece(|b| b.bare_text("/"))?;
                self.shift(s0, n.width, 0.0);
                let (d0, _, d) = self.piece(|b| b.slot(den, 0, false, false))?;
                self.shift(d0, n.width + slash.width, 0.0);
                let mut total = n;
                total.include(slash, n.width, 0.0);
                total.include(d, n.width + slash.width, 0.0);
                total.width = n.width + slash.width + d.width;
                Ok(total)
            }
            "skw" => {
                let (n0, n1, n) = self.piece(|b| b.slot(num, 1, false, false))?;
                let (d0, _, d) = self.piece(|b| b.slot(den, 1, false, true))?;
                let axis = c.axis_height;
                let gap_v = c.skewed_fraction_vertical_gap;
                let gap_h = c.skewed_fraction_horizontal_gap;
                // Numerator above the axis, denominator below, the slash
                // between them.
                let n_y = -(axis + gap_v / 2.0 + n.descent);
                let d_y = -(axis - gap_v / 2.0) + d.ascent;
                self.shift_range(n0, n1, 0.0, n_y);
                let d_x = n.width + gap_h;
                self.shift(d0, d_x, d_y);
                let top = n_y - n.ascent;
                let bottom = d_y + d.descent;
                let slash_mid_x = n.width + gap_h / 2.0;
                let slope = 0.35 * (bottom - top);
                let thickness = self.rule_thickness(&c);
                self.plan.shapes.push(MathShape {
                    commands: vec![
                        MathPathCommand::Move(slash_mid_x - slope / 2.0, bottom),
                        MathPathCommand::Line(slash_mid_x + slope / 2.0, top),
                    ],
                    stroke: Some(thickness),
                    color: self.style.color,
                });
                Ok(MathMetrics {
                    width: d_x + d.width,
                    ascent: -top,
                    descent: bottom,
                })
            }
            _ => {
                let bar = kind != "noBar";
                let (n0, n1, n) = self.piece(|b| b.slot(num, level_up, false, false))?;
                let (d0, _, d) = self.piece(|b| b.slot(den, level_up, false, true))?;
                let axis = c.axis_height;
                let t = if bar { self.rule_thickness(&c) } else { 0.0 };
                let (up, down) = if bar {
                    let up = if display {
                        c.fraction_numerator_display_shift_up
                    } else {
                        c.fraction_numerator_shift_up
                    };
                    let down = if display {
                        c.fraction_denominator_display_shift_down
                    } else {
                        c.fraction_denominator_shift_down
                    };
                    let gap_n = if display {
                        c.fraction_numerator_display_gap_min
                    } else {
                        c.fraction_numerator_gap_min
                    };
                    let gap_d = if display {
                        c.fraction_denominator_display_gap_min
                    } else {
                        c.fraction_denominator_gap_min
                    };
                    (
                        up.max(axis + t / 2.0 + gap_n + n.descent),
                        down.max(gap_d + d.ascent - (axis - t / 2.0)),
                    )
                } else {
                    let mut up = if display {
                        c.stack_top_display_shift_up
                    } else {
                        c.stack_top_shift_up
                    };
                    let mut down = if display {
                        c.stack_bottom_display_shift_down
                    } else {
                        c.stack_bottom_shift_down
                    };
                    let gap = if display {
                        c.stack_display_gap_min
                    } else {
                        c.stack_gap_min
                    };
                    let actual = (up - n.descent) - (d.ascent - down);
                    if actual < gap {
                        up += (gap - actual) / 2.0;
                        down += (gap - actual) / 2.0;
                    }
                    (up, down)
                };
                let pad = self.size() * 0.12;
                let width = n.width.max(d.width) + 2.0 * pad;
                self.shift_range(n0, n1, (width - n.width) / 2.0, -up);
                self.shift(d0, (width - d.width) / 2.0, down);
                let mut total = MathMetrics {
                    width,
                    ascent: 0.0,
                    descent: 0.0,
                };
                if bar {
                    self.plan.rules.push(MathRule {
                        x: pad * 0.5,
                        y: -axis,
                        width: width - pad,
                        thickness: t,
                        color: self.style.color,
                    });
                    total.ascent = axis + t / 2.0;
                    total.descent = (-(axis - t / 2.0)).max(0.0);
                }
                total.include(n, 0.0, -up);
                total.include(d, 0.0, down);
                total.width = width;
                Ok(total)
            }
        }
    }

    pub(crate) fn radical(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let pr = property_element(e, "radPr");
        let degree = child(e, Degree)
            .filter(|_| !on(pr, "degHide"))
            .filter(|(_, d)| d.children.iter().any(|c| !is_skipped(c) && has_content(c)));
        let c = self.constants();
        let (deg0, deg1, deg) = match degree {
            Some(found) => self.piece(|b| b.slot(Some(found), 2, false, false))?,
            None => {
                let m = self.mark();
                (m, m, MathMetrics::default())
            }
        };
        let (b0, b1, base) = self.piece(|b| b.slot(child(e, Base), 0, b.display, true))?;
        let t = c.radical_rule_thickness.max(0.5);
        let mut gap = if self.display {
            c.radical_display_vertical_gap
        } else {
            c.radical_vertical_gap
        };
        let needed = base.ascent + base.descent + gap + t;
        // The sign: the face's √ grown to the base, or a drawn one.
        let (sign0, _, sign_glyph) = self.piece(|b| b.bare_text("√"))?;
        let index = sign0.glyphs;
        let natural_h = self.plan.glyphs[index].ascent + self.plan.glyphs[index].descent;
        let mut sign_width = self.plan.glyphs[index].width;
        let sign_center = (base.descent - base.ascent - gap - t) / 2.0;
        let stretched = self.plan_stretch(index, Axis::Vertical, needed, sign_center)?;
        let font_sign = stretched.is_some() || (self.font_bound() && natural_h >= needed);
        let (sign_top, sign_bottom);
        if let Some(st) = &stretched {
            // Centred on the span it must cover, a taller sign shares its
            // excess above and below, as TeX shares the extra gap.
            sign_width = st.width;
            sign_top = -st.ascent;
            sign_bottom = st.descent;
        } else if font_sign {
            let g = &self.plan.glyphs[index];
            if natural_h > needed {
                gap += (natural_h - needed) / 2.0;
            }
            // Lift the glyph so its top meets the vinculum.
            let lift = g.ascent - (base.ascent + gap + t);
            self.plan.glyphs[index].baseline = lift;
            sign_top = -(base.ascent + gap + t);
            sign_bottom = sign_top + natural_h;
        } else {
            sign_top = -(base.ascent + gap + t);
            sign_bottom = base.descent + t;
            sign_width = sign_glyph.width.max(self.size() * 0.55);
        }
        // The degree sits over the sign's left arm.
        let kern_before = c.radical_kern_before_degree;
        let kern_after = c.radical_kern_after_degree;
        let sign_x = if degree.is_some() {
            (kern_before + deg.width + kern_after).max(0.0)
        } else {
            0.0
        };
        let deg_y =
            sign_bottom - c.radical_degree_bottom_raise * (sign_bottom - sign_top) - deg.descent;
        self.shift_range(deg0, deg1, kern_before, deg_y);
        let base_x = sign_x + sign_width;
        self.shift_range(b0, b1, base_x, 0.0);
        let rule_end = base_x + base.width + self.size() * 0.05;
        let color = self.style.color;
        if let Some(st) = stretched {
            self.plan.glyphs[index].x = sign_x;
            self.emit_stretch(index, st);
        } else if font_sign {
            self.plan.glyphs[index].x = sign_x;
        } else {
            let g = &mut self.plan.glyphs[index];
            g.visible = false;
            g.x = sign_x;
            g.width = sign_width;
            let commands = shapes::radical(sign_x, sign_top, sign_bottom, sign_width, rule_end, t);
            self.plan.shapes.push(MathShape {
                commands,
                stroke: Some(t),
                color,
            });
        }
        if font_sign {
            self.plan.rules.push(MathRule {
                x: base_x,
                y: sign_top + t / 2.0,
                width: rule_end - base_x,
                thickness: t,
                color,
            });
        }
        let mut total = MathMetrics {
            width: rule_end,
            ascent: -sign_top + c.radical_extra_ascender,
            descent: sign_bottom.max(base.descent),
        };
        if degree.is_some() {
            total.include(deg, kern_before, deg_y);
        }
        total.width = rule_end;
        Ok(total)
    }

    /// `m:sSub`, `m:sSup`, `m:sSubSup`.
    pub(crate) fn scripts(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let base_start = self.mark();
        let base = self.slot(child(e, Base), 0, self.display, self.cramped)?;
        let base_end = self.mark();
        let italic = self.lone_italic_in(base_start, base_end);
        let lower = if e.kind == Superscript {
            None
        } else {
            child(e, SubArgument)
        };
        let upper = if e.kind == Subscript {
            None
        } else {
            child(e, SuperArgument)
        };
        let lower = self.script_piece(lower, true)?;
        let upper = self.script_piece(upper, self.cramped)?;
        self.place_scripts(
            base_start, base_end, base, italic, upper, lower, false, true,
        )
    }

    /// The italic correction of a base that is one glyph.
    fn lone_italic_in(&self, start: Mark, end: Mark) -> f64 {
        if end.glyphs - start.glyphs != 1 || end.rules != start.rules || end.shapes != start.shapes
        {
            return 0.0;
        }
        #[cfg(feature = "opentype-math")]
        if let Some(g) = &self.plan.glyphs[start.glyphs].font_glyph {
            return g.italic_correction_px;
        }
        0.0
    }

    fn script_piece(
        &mut self,
        found: Option<(usize, &'a MathElement)>,
        cramped: bool,
    ) -> Result<Option<(Mark, Mark, MathMetrics)>, C::Error> {
        if found.is_none() {
            return Ok(None);
        }
        Ok(Some(self.piece(|b| b.slot(found, 1, false, cramped))?))
    }

    /// Places scripts or limits around a base laid out at the origin.
    #[allow(clippy::too_many_arguments)]
    fn place_scripts(
        &mut self,
        base_start: Mark,
        base_end: Mark,
        base: MathMetrics,
        italic: f64,
        upper: Option<(Mark, Mark, MathMetrics)>,
        lower: Option<(Mark, Mark, MathMetrics)>,
        nary: bool,
        side: bool,
    ) -> Result<MathMetrics, C::Error> {
        let c = self.constants();
        let (up, down) = if side {
            script_shifts(
                &c,
                self.cramped,
                base,
                upper.map(|s| s.2),
                lower.map(|s| s.2),
            )
        } else {
            limit_shifts(&c, base, upper.map(|s| s.2), lower.map(|s| s.2))
        };
        let upper_x = upper.map_or(0.0, |s| {
            if side {
                base.width + if nary { 0.0 } else { italic }
            } else {
                (base.width - s.2.width) / 2.0 + italic / 2.0
            }
        });
        let lower_x = lower.map_or(0.0, |s| {
            if side {
                base.width - if nary { italic } else { 0.0 }
            } else {
                (base.width - s.2.width) / 2.0 - italic / 2.0
            }
        });
        // Long limits may extend before the operator; the advance takes them.
        let origin = -0.0_f64.min(upper_x).min(lower_x);
        self.shift_range(base_start, base_end, origin, 0.0);
        let mut total = MathMetrics::default();
        total.include(base, origin, 0.0);
        if let Some((s0, s1, m)) = upper {
            self.shift_range(s0, s1, upper_x + origin, -up);
            total.include(m, upper_x + origin, -up);
        }
        if let Some((s0, s1, m)) = lower {
            self.shift_range(s0, s1, lower_x + origin, down);
            total.include(m, lower_x + origin, down);
        }
        if side && (upper.is_some() || lower.is_some()) {
            total.width += c.space_after_script;
        }
        Ok(total)
    }

    /// `m:sPre`: scripts before the base, right-aligned against it.
    pub(crate) fn prescripts(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let lower = self.script_piece(child(e, SubArgument), true)?;
        let upper = self.script_piece(child(e, SuperArgument), self.cramped)?;
        let (b0, b1, base) = self.piece(|b| b.slot(child(e, Base), 0, b.display, b.cramped))?;
        let c = self.constants();
        let (up, down) = script_shifts(
            &c,
            self.cramped,
            base,
            upper.map(|s| s.2),
            lower.map(|s| s.2),
        );
        let width = upper
            .map_or(0.0, |s| s.2.width)
            .max(lower.map_or(0.0, |s| s.2.width));
        let mut total = MathMetrics::default();
        if let Some((s0, s1, m)) = upper {
            self.shift_range(s0, s1, width - m.width, -up);
            total.include(m, width - m.width, -up);
        }
        if let Some((s0, s1, m)) = lower {
            self.shift_range(s0, s1, width - m.width, down);
            total.include(m, width - m.width, down);
        }
        let x = width + c.space_after_script;
        self.shift_range(b0, b1, x, 0.0);
        total.include(base, x, 0.0);
        Ok(total)
    }

    pub(crate) fn nary(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let pr = property_element(e, "naryPr");
        let symbol = nary_symbol(e).unwrap_or_else(|| "∫".to_string());
        let integral = symbol
            .chars()
            .next()
            .is_some_and(|ch| INTEGRALS.contains(ch));
        let side = match property(pr, "limLoc") {
            Some("subSup") => true,
            Some("undOvr") => false,
            _ => {
                if integral {
                    self.options.settings.integral_limits_side
                } else {
                    !self.options.settings.nary_limits_over
                }
            }
        };
        // Inline math keeps its operators small, limits at the side.
        let side = side || !self.display;
        let grow = on(pr, "grow");
        // The operator: the face's display variant in display math, or a
        // larger glyph centred on the axis when no face is bound.
        let bound = self.font_bound();
        let saved_style = self.style;
        let enlarge = if self.display && !bound && !self.options.uniform {
            if integral {
                1.6
            } else {
                1.4
            }
        } else {
            1.0
        };
        self.style.size_px *= enlarge;
        #[cfg(feature = "opentype-math")]
        {
            self.display_operator = self.display;
        }
        let saved_prev = self.prev.take();
        let (sym0, sym1, mut sym) = self.piece(|b| b.text(&symbol))?;
        self.prev = saved_prev;
        #[cfg(feature = "opentype-math")]
        {
            self.display_operator = false;
        }
        self.style = saved_style;
        let axis = self.constants().axis_height;
        // The operator is centred on the math axis, as Word sets it: an
        // inline ∫ in Cambria Math sits half a point lower than its glyph's
        // own placement (`math` oracle).
        if !self.options.uniform && sym1.glyphs > sym0.glyphs {
            let shift = (sym.ascent - sym.descent) / 2.0 - axis;
            self.shift_range(sym0, sym1, 0.0, shift);
            sym.ascent -= shift;
            sym.descent += shift;
            for g in &mut self.plan.glyphs[sym0.glyphs..sym1.glyphs] {
                g.ascent -= shift;
                g.descent += shift;
            }
        }
        let italic = self.lone_italic_in(sym0, sym1);
        let upper = if on(pr, "supHide") {
            None
        } else {
            self.script_piece(child(e, SuperArgument), self.cramped)?
        };
        let lower = if on(pr, "subHide") {
            None
        } else {
            self.script_piece(child(e, SubArgument), true)?
        };
        let (b0, b1, body) = self.piece(|b| b.slot(child(e, Base), 0, b.display, b.cramped))?;
        // m:grow: the operator grows to the height of what it applies to.
        if grow && sym1.glyphs > sym0.glyphs {
            let target = (body.ascent - axis).max(body.descent + axis) * 2.0;
            if target > sym.ascent + sym.descent {
                if let Some(st) = self.plan_stretch(sym0.glyphs, Axis::Vertical, target, -axis)? {
                    sym = MathMetrics {
                        width: st.width,
                        ascent: st.ascent,
                        descent: st.descent,
                    };
                    self.emit_stretch(sym0.glyphs, st);
                }
            }
        }
        let mut total = self.place_scripts(sym0, sym1, sym, italic, upper, lower, true, side)?;
        // The operand follows the operator and its limits.
        let x = total.width + self.size() * 0.1;
        self.shift_range(b0, b1, x, 0.0);
        total.include(body, x, 0.0);
        Ok(total)
    }

    pub(crate) fn delimiter(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let pr = property_element(e, "dPr");
        let begin = property(pr, "begChr").unwrap_or("(");
        let end = property(pr, "endChr").unwrap_or(")");
        let separator = property(pr, "sepChr").unwrap_or("|");
        let grow = on_unless_off(pr, "grow");
        let matched = property(pr, "shp") == Some("match");
        let c = self.constants();
        let thin = self.size() * 3.0 / 18.0;
        // Pieces in order: open, argument, separator, argument, …, close.
        let mut pieces: Vec<(Mark, Mark, MathMetrics, bool)> = Vec::new();
        if !begin.is_empty() {
            let (s0, s1, m) = self.piece(|b| b.bare_text(begin))?;
            pieces.push((s0, s1, m, true));
        }
        let args: Vec<_> = children_of(e, Base).collect();
        let mut content = MathMetrics::default();
        for (i, found) in args.iter().enumerate() {
            if i > 0 && !separator.is_empty() {
                let (s0, s1, m) = self.piece(|b| b.bare_text(separator))?;
                pieces.push((s0, s1, m, true));
            }
            let (s0, s1, m) = self.piece(|b| b.slot(Some(*found), 0, b.display, b.cramped))?;
            content.ascent = content.ascent.max(m.ascent);
            content.descent = content.descent.max(m.descent);
            pieces.push((s0, s1, m, false));
        }
        if !end.is_empty() {
            let (s0, s1, m) = self.piece(|b| b.bare_text(end))?;
            pieces.push((s0, s1, m, true));
        }
        // How tall the delimiters must be.
        let axis = c.axis_height;
        let (target, center) = if matched {
            (
                content.ascent + content.descent,
                (content.descent - content.ascent) / 2.0,
            )
        } else {
            (
                2.0 * (content.ascent - axis).max(content.descent + axis),
                -axis,
            )
        };
        // A delimiter may fall a little short of what it encloses, as TeX's
        // delimiter factor and shortfall allow: brackets around one line of
        // text keep their ordinary size.
        let target = (target * 0.901).max(target - self.size() * 0.5);
        let mut planned = Vec::new();
        for (i, piece) in pieces.iter_mut().enumerate() {
            if !piece.3 || !grow || piece.1.glyphs == piece.0.glyphs {
                continue;
            }
            if let Some(st) = self.plan_stretch(piece.0.glyphs, Axis::Vertical, target, center)? {
                piece.2 = MathMetrics {
                    width: st.width,
                    ascent: st.ascent,
                    descent: st.descent,
                };
                planned.push((i, st));
            }
        }
        // Lay the pieces side by side; separators get a thin space each side.
        let mut x = 0.0;
        let mut total = MathMetrics::default();
        let count = pieces.len();
        for (i, piece) in pieces.iter().enumerate() {
            let separator_piece = piece.3 && i > 0 && i + 1 < count;
            if separator_piece {
                x += thin;
            }
            self.shift_range(piece.0, piece.1, x, 0.0);
            total.include(piece.2, x, 0.0);
            x += piece.2.width;
            if separator_piece {
                x += thin;
            }
        }
        for (i, st) in planned {
            let index = pieces[i].0.glyphs;
            self.emit_stretch(index, st);
        }
        total.width = x;
        Ok(total)
    }

    pub(crate) fn function(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let (_, _, name) =
            self.piece(|b| b.slot(child(e, FunctionName), 0, b.display, b.cramped))?;
        let body_found = child(e, Base);
        // A thin space after the name, unless a bracket follows it.
        let opens = body_found.is_some_and(|(_, body)| starts_with_open(body));
        let space = if opens { 0.0 } else { self.size() * 3.0 / 18.0 };
        let (b0, _, body) = self.piece(|b| b.slot(body_found, 0, b.display, b.cramped))?;
        let x = name.width + space;
        self.shift(b0, x, 0.0);
        let mut total = name;
        total.include(body, x, 0.0);
        total.width = x + body.width;
        Ok(total)
    }

    pub(crate) fn accent(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let pr = property_element(e, "accPr");
        let chr = property(pr, "chr")
            .filter(|v| !v.is_empty())
            .unwrap_or("\u{0302}");
        let ch = chr.chars().next().unwrap_or('\u{0302}');
        let base_start = self.mark();
        let base = self.slot(child(e, Base), 0, self.display, true)?;
        let base_end = self.mark();
        let c = self.constants();
        let italic = self.lone_italic_in(base_start, base_end);
        let lone = base_end.glyphs - base_start.glyphs == 1;
        // Where the accent centres: the glyph's attachment, or its middle
        // nudged by the slant.
        let attach = {
            #[cfg(feature = "opentype-math")]
            {
                let from_font = if lone {
                    let g = &self.plan.glyphs[base_start.glyphs];
                    self.context
                        .top_accent_attachment(&self.plan.text[g.text_range.clone()], g.style)
                        .map(|a| g.x + a)
                } else {
                    None
                };
                from_font.unwrap_or(base.width / 2.0 + italic / 2.0)
            }
            #[cfg(not(feature = "opentype-math"))]
            {
                let _ = lone;
                base.width / 2.0 + italic / 2.0
            }
        };
        let wide = !lone || base.width > self.size() * 0.9;
        let size = self.size();
        let gap = size * 0.06;
        let raise = (base.ascent - c.accent_base_height).max(0.0);
        let mut total = base;
        if self.font_bound() && !wide {
            // The face's combining mark, raised over a tall base.
            let (a0, a1, accent) = self.piece(|b| b.bare_text(chr))?;
            let g = &self.plan.glyphs[a0.glyphs];
            let ink_center = {
                #[cfg(feature = "opentype-math")]
                {
                    g.font_glyph.as_ref().map_or(g.width / 2.0, |f| {
                        (f.geometry.ink_px.x_min + f.geometry.ink_px.x_max) / 2.0
                    })
                }
                #[cfg(not(feature = "opentype-math"))]
                {
                    g.width / 2.0
                }
            };
            let dx = attach - ink_center;
            self.shift_range(a0, a1, dx, -raise);
            // The mark's advance is not width: the base's is.
            self.plan.glyphs[a0.glyphs].width = 0.0;
            total.ascent = total.ascent.max(accent.ascent + raise);
            return Ok(total);
        }
        // A drawn accent: a hat, a tilde, an arrow or a bar over the base,
        // as wide as the base when the base is wide.
        let accent_height = match ch {
            '\u{0307}' | '\u{0308}' | '\u{20DB}' | '\u{030A}' => size * 0.12,
            '\u{0304}' | '\u{0305}' => size * 0.04,
            '\u{20D6}' | '\u{20D7}' | '\u{20E1}' | '\u{20D0}' | '\u{20D1}' => size * 0.22,
            _ => size * 0.16,
        };
        let natural = match ch {
            '\u{0304}' | '\u{0305}' | '\u{20D6}' | '\u{20D7}' | '\u{20E1}' | '\u{20D0}'
            | '\u{20D1}' => base.width,
            '\u{0307}' | '\u{030A}' => size * 0.12,
            '\u{0308}' => size * 0.3,
            '\u{20DB}' => size * 0.45,
            _ => size * 0.45,
        };
        let width = if wide {
            base.width.max(natural)
        } else {
            natural
        };
        let x = if wide {
            (base.width - width) / 2.0
        } else {
            attach - width / 2.0
        };
        let bottom = -(base.ascent.max(c.accent_base_height * 0.95) + gap);
        let top = bottom - accent_height;
        let drawn = self.horizontal_mark(ch, x, top, width, accent_height)?;
        if drawn == 0.0 {
            // No drawn form: the spacing character, set over the base.
            let shown = super::alphabet::spacing_accent(ch).to_string();
            let (a0, a1, accent) = self.piece(|b| b.bare_text(&shown))?;
            let dy = bottom - accent.descent.min(size * 0.1);
            self.shift_range(
                a0,
                a1,
                attach - accent.width / 2.0,
                dy - accent.ascent * 0.4,
            );
            self.plan.glyphs[a0.glyphs].width = 0.0;
            total.ascent = total.ascent.max(-(dy - accent.ascent * 1.4));
            return Ok(total);
        }
        total.ascent = total.ascent.max(-top);
        total.width = total.width.max(x + width);
        Ok(total)
    }

    pub(crate) fn bar(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let pr = property_element(e, "barPr");
        let top = property(pr, "pos") == Some("top");
        let base = self.slot(child(e, Base), 0, self.display, top || self.cramped)?;
        let c = self.constants();
        let mut total = base;
        if top {
            let t = c.overbar_rule_thickness.max(0.5);
            let y = -(base.ascent + c.overbar_vertical_gap + t / 2.0);
            self.plan.rules.push(MathRule {
                x: 0.0,
                y,
                width: base.width,
                thickness: t,
                color: self.style.color,
            });
            total.ascent = -y + t / 2.0 + c.overbar_extra_ascender;
        } else {
            let t = c.underbar_rule_thickness.max(0.5);
            let y = base.descent + c.underbar_vertical_gap + t / 2.0;
            self.plan.rules.push(MathRule {
                x: 0.0,
                y,
                width: base.width,
                thickness: t,
                color: self.style.color,
            });
            total.descent = y + t / 2.0 + c.underbar_extra_descender;
        }
        Ok(total)
    }

    pub(crate) fn group_char(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let pr = property_element(e, "groupChrPr");
        let chr = property(pr, "chr").filter(|v| !v.is_empty()).unwrap_or("⏟");
        let ch = chr.chars().next().unwrap_or('⏟');
        let top = property(pr, "pos") == Some("top");
        let vert = property(pr, "vertJc");
        let (b0, b1, base) = self.piece(|b| b.slot(child(e, Base), 0, b.display, b.cramped))?;
        let c = self.constants();
        let size = self.size();
        let height = size * 0.3;
        let (mark_top, gap) = if top {
            let gap = c.stretch_stack_gap_above_min.max(size * 0.05);
            (-(base.ascent + gap + height), gap)
        } else {
            let gap = c.stretch_stack_gap_below_min.max(size * 0.05);
            (base.descent + gap, gap)
        };
        let _ = gap;
        let drawn = self.horizontal_mark(ch, 0.0, mark_top, base.width, height)?;
        let mut total = base;
        if top {
            total.ascent = total.ascent.max(-mark_top);
        } else {
            total.descent = total.descent.max(mark_top + drawn.max(height));
        }
        // vertJc moves the baseline to the character when it says so.
        let shift = match (top, vert) {
            (false, Some("bot")) => -(mark_top + drawn.max(height)),
            (true, Some("top")) => -mark_top,
            _ => 0.0,
        };
        if shift != 0.0 {
            let start = b0;
            let _ = b1;
            self.shift(start, 0.0, shift);
            total.ascent -= shift;
            total.descent += shift;
        }
        Ok(total)
    }

    /// `m:limLow` and `m:limUpp`.
    pub(crate) fn limit(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let (b0, b1, base) = self.piece(|b| b.slot(child(e, Base), 0, b.display, b.cramped))?;
        let lower = e.kind == LimitLower;
        let (l0, l1, lim) = self.piece(|b| b.slot(child(e, Limit), 1, false, lower))?;
        let c = self.constants();
        let width = base.width.max(lim.width);
        self.shift_range(b0, b1, (width - base.width) / 2.0, 0.0);
        let y = if lower {
            base.descent
                + c.lower_limit_baseline_drop_min
                    .max(c.lower_limit_gap_min + lim.ascent)
        } else {
            -(base.ascent
                + c.upper_limit_baseline_rise_min
                    .max(c.upper_limit_gap_min + lim.descent))
        };
        self.shift_range(l0, l1, (width - lim.width) / 2.0, y);
        let mut total = MathMetrics::default();
        total.include(base, (width - base.width) / 2.0, 0.0);
        total.include(lim, (width - lim.width) / 2.0, y);
        total.width = width;
        Ok(total)
    }

    /// `m:box`: its base, grouped; `m:diff` puts a thin space before it.
    pub(crate) fn boxed(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        let pr = property_element(e, "boxPr");
        let (b0, _, base) =
            self.piece(|b| b.slot(child(e, MathElementKind::Base), 0, b.display, b.cramped))?;
        if on(pr, "diff") {
            let space = self.size() * 3.0 / 18.0;
            self.shift(b0, space, 0.0);
            let mut total = base;
            total.width += space;
            return Ok(total);
        }
        Ok(base)
    }

    pub(crate) fn border_box(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        let pr = property_element(e, "borderBoxPr");
        let (b0, _, base) =
            self.piece(|b| b.slot(child(e, MathElementKind::Base), 0, b.display, b.cramped))?;
        let c = self.constants();
        let t = self.rule_thickness(&c);
        let pad = self.size() * 0.15;
        self.shift(b0, pad, 0.0);
        let left = t / 2.0;
        let right = base.width + 2.0 * pad - t / 2.0;
        let top = -(base.ascent + pad);
        let bottom = base.descent + pad;
        let mut commands = Vec::new();
        let mut side = |show: bool, a: (f64, f64), z: (f64, f64)| {
            if show {
                commands.push(MathPathCommand::Move(a.0, a.1));
                commands.push(MathPathCommand::Line(z.0, z.1));
            }
        };
        side(!on(pr, "hideTop"), (0.0, top), (right + t / 2.0, top));
        side(!on(pr, "hideBot"), (0.0, bottom), (right + t / 2.0, bottom));
        side(!on(pr, "hideLeft"), (left, top), (left, bottom));
        side(!on(pr, "hideRight"), (right, top), (right, bottom));
        let axis = c.axis_height;
        let mid_x = (left + right) / 2.0;
        side(on(pr, "strikeH"), (left, -axis), (right, -axis));
        side(on(pr, "strikeV"), (mid_x, top), (mid_x, bottom));
        side(on(pr, "strikeBLTR"), (left, bottom), (right, top));
        side(on(pr, "strikeTLBR"), (left, top), (right, bottom));
        if !commands.is_empty() {
            self.plan.shapes.push(MathShape {
                commands,
                stroke: Some(t),
                color: self.style.color,
            });
        }
        Ok(MathMetrics {
            width: base.width + 2.0 * pad,
            ascent: -top + t / 2.0,
            descent: bottom + t / 2.0,
        })
    }

    pub(crate) fn phantom(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        let pr = property_element(e, "phantPr");
        let (b0, _, base) =
            self.piece(|b| b.slot(child(e, MathElementKind::Base), 0, b.display, b.cramped))?;
        if !on_unless_off(pr, "show") {
            for g in &mut self.plan.glyphs[b0.glyphs..] {
                g.visible = false;
            }
            self.plan.rules.truncate(b0.rules);
            self.plan.shapes.truncate(b0.shapes);
        }
        let mut total = base;
        if on(pr, "zeroWid") {
            total.width = 0.0;
        }
        if on(pr, "zeroAsc") {
            total.ascent = 0.0;
        }
        if on(pr, "zeroDesc") {
            total.descent = 0.0;
        }
        Ok(total)
    }

    /// Row spacing for equation arrays and matrices (`rSpRule`, `rSp`).
    fn row_gap(&self, pr: Option<&MathElement>, rule: &str, value: &str) -> f64 {
        let size = self.size();
        let single = size * 0.25;
        let twips = || {
            property(pr, value)
                .and_then(|v| v.parse::<f64>().ok())
                .map(|v| v / 20.0 * 96.0 / 72.0)
                .unwrap_or(0.0)
        };
        match property(pr, rule) {
            Some("1") => single + size * 0.5,
            Some("2") => single + size,
            Some("3") => twips(),
            Some("4") => {
                // Multiples of a line, in halves.
                let halves = property(pr, value)
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(2.0);
                single + size * (halves / 2.0 - 1.0).max(0.0)
            }
            _ => single,
        }
    }

    /// Stacks rows (each laid out at the origin) and centres them as
    /// `baseJc` says. Returns the total.
    fn stack_rows(
        &mut self,
        rows: &[(Mark, Mark, MathMetrics)],
        gap: f64,
        base_jc: Option<&str>,
    ) -> MathMetrics {
        let size = self.size();
        let mut baseline = 0.0;
        let mut baselines = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            if i > 0 {
                let prev = rows[i - 1].2;
                baseline += (prev.descent + row.2.ascent + gap).max(size * 1.0);
            }
            baselines.push(baseline);
        }
        let first_ascent = rows.first().map_or(0.0, |r| r.2.ascent);
        let last_descent = rows.last().map_or(0.0, |r| r.2.descent);
        let top = -first_ascent;
        let bottom = baseline + last_descent;
        let axis = self.constants().axis_height;
        let shift = match base_jc {
            Some("top") => 0.0,
            Some("bottom") | Some("bot") => -baseline,
            _ => -axis - (top + bottom) / 2.0,
        };
        let mut total = MathMetrics::default();
        for (row, y) in rows.iter().zip(baselines) {
            self.shift_range(row.0, row.1, 0.0, y + shift);
            total.include(row.2, 0.0, y + shift);
        }
        total
    }

    pub(crate) fn equation_array(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        let pr = property_element(e, "eqArrPr");
        let mut rows = Vec::new();
        let mut segments: Vec<Vec<(Mark, f64)>> = Vec::new();
        for found in children_of(e, MathElementKind::Base) {
            let saved = self.amps.take();
            self.amps = Some((self.depth + 1, Vec::new()));
            let (r0, r1, m) = self.piece(|b| b.slot(Some(found), 0, b.display, b.cramped))?;
            let amps = self.amps.take().map(|a| a.1).unwrap_or_default();
            self.amps = saved;
            // Segments between the row's `&` marks.
            let mut row_segments = vec![(r0, 0.0)];
            for mark in amps {
                let x = self.plan.glyphs[mark.glyphs].x;
                row_segments.push((mark, x));
            }
            let _ = r1;
            rows.push((r0, r1, m));
            segments.push(row_segments);
        }
        // Columns: segments alternate right- and left-aligned, as `&` pairs
        // align in Word.
        let columns = segments.iter().map(Vec::len).max().unwrap_or(1);
        let mut widths = vec![0.0f64; columns];
        for (row, segs) in rows.iter().zip(&segments) {
            for (k, seg) in segs.iter().enumerate() {
                let end_x = segs.get(k + 1).map_or(row.2.width, |s| s.1);
                widths[k] = widths[k].max(end_x - seg.1);
            }
        }
        let starts: Vec<f64> = widths
            .iter()
            .scan(0.0, |x, w| {
                let s = *x;
                *x += w;
                Some(s)
            })
            .collect();
        let total_width: f64 = widths.iter().sum();
        for (i, (row, segs)) in rows.iter_mut().zip(&segments).enumerate() {
            let _ = i;
            for (k, seg) in segs.iter().enumerate() {
                let end_x = segs.get(k + 1).map_or(row.2.width, |s| s.1);
                let seg_end = segs.get(k + 1).map_or(row.1, |s| s.0);
                let w = end_x - seg.1;
                let target = if k % 2 == 0 && columns > 1 {
                    starts[k] + widths[k] - w
                } else {
                    starts[k]
                };
                self.shift_range(seg.0, seg_end, target - seg.1, 0.0);
            }
            if columns > 1 {
                row.2.width = total_width;
            }
        }
        let gap = self.row_gap(pr, "rSpRule", "rSp");
        let mut total = self.stack_rows(&rows, gap, property(pr, "baseJc"));
        total.width = rows.iter().map(|r| r.2.width).fold(0.0, f64::max);
        Ok(total)
    }

    pub(crate) fn matrix(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        use MathElementKind::*;
        let pr = property_element(e, "mPr");
        let hide_placeholders = on(pr, "plcHide");
        // Column justification from m:mcs.
        let mut column_jc: Vec<&str> = Vec::new();
        if let Some(mcs) = pr.and_then(|p| p.children.iter().find(|c| c.local_name() == "mcs")) {
            for mc in mcs.children.iter().filter(|c| c.local_name() == "mc") {
                let mc_pr = property_element(mc, "mcPr");
                let count = property(mc_pr, "count")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 64);
                let jc = property(mc_pr, "mcJc").unwrap_or("center");
                column_jc.extend(std::iter::repeat_n(jc, count));
            }
        }
        let mut cells: Vec<Vec<(Mark, Mark, MathMetrics)>> = Vec::new();
        for (row_index, row) in children_of(e, MatrixRow) {
            self.path.push(row_index as u16);
            let mut row_cells = Vec::new();
            for found in children_of(row, Base) {
                let (s0, s1, mut m) = self.piece(|b| b.slot(Some(found), 0, false, b.cramped))?;
                if hide_placeholders
                    && !found
                        .1
                        .children
                        .iter()
                        .any(|c| !is_skipped(c) && has_content(c))
                {
                    m = MathMetrics::default();
                }
                row_cells.push((s0, s1, m));
            }
            self.path.pop();
            cells.push(row_cells);
        }
        let columns = cells.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0.0f64; columns];
        for row in &cells {
            for (k, cell) in row.iter().enumerate() {
                widths[k] = widths[k].max(cell.2.width);
            }
        }
        let size = self.size();
        let column_gap = match property(pr, "cGpRule") {
            Some("1") => size * 1.5,
            Some("2") => size * 2.0,
            Some("3") => property(pr, "cGp")
                .and_then(|v| v.parse::<f64>().ok())
                .map_or(size, |v| v / 20.0 * 96.0 / 72.0),
            Some("4") => {
                property(pr, "cGp")
                    .and_then(|v| v.parse::<f64>().ok())
                    .unwrap_or(2.0)
                    * size
                    / 2.0
            }
            _ => size,
        };
        let min_spacing = property(pr, "cSp")
            .and_then(|v| v.parse::<f64>().ok())
            .map_or(0.0, |v| v / 20.0 * 96.0 / 72.0);
        let mut starts = Vec::new();
        let mut x = 0.0;
        for (k, w) in widths.iter().enumerate() {
            starts.push(x);
            x += w.max(min_spacing);
            if k + 1 < columns {
                x += column_gap;
            }
        }
        let total_width = x;
        let mut rows = Vec::new();
        for row in &cells {
            let mut metrics = MathMetrics {
                width: total_width,
                ..MathMetrics::default()
            };
            for (k, cell) in row.iter().enumerate() {
                let slack = widths[k].max(min_spacing) - cell.2.width;
                let dx = starts[k]
                    + match column_jc.get(k).copied().unwrap_or("center") {
                        "left" => 0.0,
                        "right" => slack,
                        _ => slack / 2.0,
                    };
                self.shift_range(cell.0, cell.1, dx, 0.0);
                metrics.ascent = metrics.ascent.max(cell.2.ascent);
                metrics.descent = metrics.descent.max(cell.2.descent);
            }
            let start = row.first().map_or(self.mark(), |c| c.0);
            let end = row.last().map_or(self.mark(), |c| c.1);
            // An empty row still takes a line.
            if row.is_empty() {
                metrics.ascent = size * 0.7;
            }
            rows.push((start, end, metrics));
        }
        let gap = self.row_gap(pr, "rSpRule", "rSp");
        let mut total = self.stack_rows(&rows, gap, property(pr, "baseJc"));
        total.width = total_width;
        Ok(total)
    }

    /// The EQ field's own structures (`eq_field`): \s shifts, \a arrays,
    /// \o overstrikes, \d displacements and \l lists.
    pub(crate) fn eq_structure(&mut self, e: &'a MathElement) -> Result<MathMetrics, C::Error> {
        let points = |name: &str| -> f64 {
            e.attributes
                .get(name)
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(0.0)
                * 96.0
                / 72.0
        };
        match e.local_name() {
            "eqShift" => {
                // \s: arguments stacked, the whole raised (\up) or lowered.
                let up = points("up") - points("do");
                let mut rows = Vec::new();
                for found in children_of(e, MathElementKind::Base) {
                    rows.push(self.piece(|b| b.slot(Some(found), 0, b.display, false))?);
                }
                let mut total = MathMetrics::default();
                let mut y = 0.0;
                let mut first = true;
                for (r0, r1, m) in &rows {
                    if !first {
                        y += m.ascent + points("ai").max(0.0);
                    }
                    first = false;
                    self.shift_range(*r0, *r1, 0.0, y - up);
                    total.include(*m, 0.0, y - up);
                    y += m.descent + points("di").max(0.0);
                }
                Ok(total)
            }
            "eqArray" => {
                // \a: elements in rows of \co columns.
                let columns = e
                    .attributes
                    .get("co")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .max(1);
                let align = e.attributes.get("al").map(String::as_str).unwrap_or("c");
                let vs = points("vs");
                let hs = points("hs");
                let mut cells = Vec::new();
                for found in children_of(e, MathElementKind::Base) {
                    cells.push(self.piece(|b| b.slot(Some(found), 0, b.display, false))?);
                }
                let mut widths = vec![0.0f64; columns];
                for (i, c) in cells.iter().enumerate() {
                    widths[i % columns] = widths[i % columns].max(c.2.width);
                }
                let mut starts = Vec::new();
                let mut x = 0.0;
                for w in &widths {
                    starts.push(x);
                    x += w + hs;
                }
                let width = (x - hs).max(0.0);
                let mut rows = Vec::new();
                for chunk in cells.chunks(columns) {
                    let mut m = MathMetrics {
                        width,
                        ..MathMetrics::default()
                    };
                    for (k, c) in chunk.iter().enumerate() {
                        let slack = widths[k] - c.2.width;
                        let dx = starts[k]
                            + match align {
                                "l" => 0.0,
                                "r" => slack,
                                _ => slack / 2.0,
                            };
                        self.shift_range(c.0, c.1, dx, 0.0);
                        m.ascent = m.ascent.max(c.2.ascent);
                        m.descent = m.descent.max(c.2.descent);
                    }
                    rows.push((chunk[0].0, chunk[chunk.len() - 1].1, m));
                }
                let mut total = self.stack_rows(&rows, vs, Some("center"));
                total.width = width;
                Ok(total)
            }
            "eqOverstrike" => {
                // \o: every argument printed over the others.
                let align = e.attributes.get("al").map(String::as_str).unwrap_or("c");
                let mut pieces = Vec::new();
                for found in children_of(e, MathElementKind::Base) {
                    pieces.push(self.piece(|b| b.slot(Some(found), 0, b.display, false))?);
                }
                let width = pieces.iter().map(|p| p.2.width).fold(0.0, f64::max);
                let mut total = MathMetrics {
                    width,
                    ..MathMetrics::default()
                };
                for (p0, p1, m) in pieces {
                    let dx = match align {
                        "l" => 0.0,
                        "r" => width - m.width,
                        _ => (width - m.width) / 2.0,
                    };
                    self.shift_range(p0, p1, dx, 0.0);
                    total.include(m, dx, 0.0);
                }
                Ok(total)
            }
            "eqDisplace" => {
                // \d: move the next character forward (\fo) or back (\ba),
                // underlining the gap with \li.
                let advance = points("fo") - points("ba");
                if e.attributes.contains_key("li") && advance > 0.0 {
                    let c = self.constants();
                    self.plan.rules.push(MathRule {
                        x: 0.0,
                        y: self.size() * 0.1,
                        width: advance,
                        thickness: self.rule_thickness(&c),
                        color: self.style.color,
                    });
                }
                Ok(MathMetrics {
                    width: advance,
                    ascent: 0.0,
                    descent: 0.0,
                })
            }
            _ => {
                // \l: the arguments one after another, separated by commas.
                let mut x = 0.0;
                let mut total = MathMetrics::default();
                let args: Vec<_> = children_of(e, MathElementKind::Base).collect();
                for (i, found) in args.iter().enumerate() {
                    if i > 0 {
                        let (s0, _, comma) = self.piece(|b| b.bare_text(","))?;
                        self.shift(s0, x, 0.0);
                        total.include(comma, x, 0.0);
                        x += comma.width;
                    }
                    let (p0, _, m) = self.piece(|b| b.slot(Some(*found), 0, b.display, false))?;
                    self.shift(p0, x, 0.0);
                    total.include(m, x, 0.0);
                    x += m.width;
                }
                total.width = x;
                Ok(total)
            }
        }
    }
}

/// Whether an argument starts with a bracket (no space after a function
/// name then).
fn starts_with_open(e: &MathElement) -> bool {
    for c in &e.children {
        if is_skipped(c) {
            continue;
        }
        return match c.kind {
            MathElementKind::Delimiter => true,
            MathElementKind::Run | MathElementKind::Text => {
                let mut text = String::new();
                collect_text(c, &mut text);
                text.chars()
                    .next()
                    .is_some_and(|ch| super::alphabet::char_class(ch) == AtomClass::Open)
            }
            _ => false,
        };
    }
    false
}

/// Superscript and subscript shifts from the MATH constants.
fn script_shifts(
    c: &MathConstants,
    cramped: bool,
    base: MathMetrics,
    upper: Option<MathMetrics>,
    lower: Option<MathMetrics>,
) -> (f64, f64) {
    let mut up = 0.0;
    let mut down = 0.0;
    if let Some(upper) = upper {
        let standard = if cramped {
            c.superscript_shift_up_cramped
        } else {
            c.superscript_shift_up
        };
        up = standard
            .max(c.superscript_bottom_min + upper.descent)
            .max(base.ascent - c.superscript_baseline_drop_max);
    }
    if let Some(lower) = lower {
        down = c
            .subscript_shift_down
            .max(lower.ascent - c.subscript_top_max)
            .max(base.descent + c.subscript_baseline_drop_min);
    }
    if let (Some(upper), Some(lower)) = (upper, lower) {
        let gap = up - upper.descent + down - lower.ascent;
        let missing = (c.sub_superscript_gap_min - gap).max(0.0);
        let raise = (c.superscript_bottom_max_with_subscript - (up - upper.descent))
            .max(0.0)
            .min(missing);
        up += raise;
        down += missing - raise;
    }
    (up, down)
}

/// Limits over and under an operator.
fn limit_shifts(
    c: &MathConstants,
    base: MathMetrics,
    upper: Option<MathMetrics>,
    lower: Option<MathMetrics>,
) -> (f64, f64) {
    (
        upper.map_or(0.0, |m| {
            base.ascent
                + c.upper_limit_baseline_rise_min
                    .max(c.upper_limit_gap_min + m.descent)
        }),
        lower.map_or(0.0, |m| {
            base.descent
                + c.lower_limit_baseline_drop_min
                    .max(c.lower_limit_gap_min + m.ascent)
        }),
    )
}
