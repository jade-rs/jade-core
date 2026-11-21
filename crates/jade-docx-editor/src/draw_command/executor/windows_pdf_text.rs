//! Physical font sizing from independently captured Windows Word PDF text.
//!
//! The 22pt Calibri tracking controls use 21.96pt in Word's PDF (183 pixels
//! at 600dpi). Screen EMFs retain logical advances. Only physical drawing is
//! quantized here; line breaking, source tracking and Mac text stay logical.
use super::{FontStateTracker, TextStyle};
use crate::draw_command::GlyphRunStep;
use jade_painter::Painter;
use std::rc::Rc;

pub(crate) fn eligible<P: Painter>(context: &P, text: &str, style: &TextStyle) -> bool {
    let Some(dpi) = context.text_output_dpi() else {
        return false;
    };
    jade_docx_layout::line_metrics::word_layout_platform()
        == jade_docx_layout::line_metrics::WordLayoutPlatform::Windows
        && (style.text_scale_x - 1.0).abs() < f64::EPSILON
        && !style.outline
        && !style.shadow
        && !style.emboss
        && !style.imprint
        && style.effects.is_none()
        && !text.is_empty()
        && text.bytes().all(|value| (b' '..=b'~').contains(&value))
        && ((style.letter_spacing_px != 0.0
            && style.enable_pair_kerning
            && regular_calibri(&style.font_descriptor_latin_ascii).is_some())
            || realized_print_parts(&style.font_descriptor_latin_ascii, dpi).is_some())
}

// Source-only physical em realization on the existing print-device grid.
// Keep precomputed glyph origins and preserve the tracked Calibri path.
// Actual corpus font-size and coordinate assertions are required for promotion.
fn realized_print_parts(descriptor: &str, dpi: f64) -> Option<(&str, &str, f64)> {
    if !dpi.is_finite() || dpi <= 0.0 {
        return None;
    }
    for token in descriptor.split_whitespace() {
        let Some(number) = token
            .strip_suffix("px")
            .or_else(|| token.strip_suffix("pt"))
        else {
            continue;
        };
        let size = number.parse::<f64>().ok()?;
        let source_em = if token.ends_with("pt") {
            size * 96.0 / 72.0
        } else {
            size
        };
        if !source_em.is_finite() || source_em <= 0.0 {
            return None;
        }
        let position = descriptor.find(token)?;
        let prefix = &descriptor[..position];
        let suffix = &descriptor[position + token.len()..];
        let family = suffix
            .split(',')
            .next()?
            .trim()
            .trim_matches(&['\'', '"'][..]);
        if family.is_empty() {
            return None;
        }
        let realized = (source_em * dpi / 96.0).round() * 96.0 / dpi;
        return (realized.is_finite() && realized > 0.0 && (realized - source_em).abs() > 1e-9)
            .then_some((prefix, suffix, realized));
    }
    None
}

fn regular_calibri(descriptor: &str) -> Option<(&str, &str)> {
    for token in descriptor.split_whitespace() {
        let numeric = token
            .strip_suffix("px")
            .or_else(|| token.strip_suffix("pt"));
        let Some(numeric) = numeric else { continue };
        let size = numeric.parse::<f64>().ok()?;
        if !size.is_finite() || size <= 0.0 {
            return None;
        }
        let position = descriptor.find(token)?;
        let prefix = &descriptor[..position];
        if !prefix
            .split_whitespace()
            .all(|value| matches!(value, "normal" | "400"))
        {
            return None;
        }
        let suffix = &descriptor[position + token.len()..];
        let family = suffix
            .split(',')
            .next()?
            .trim()
            .trim_matches(&['\'', '"'][..]);
        return (family == "Calibri").then_some((prefix, suffix));
    }
    None
}

#[allow(clippy::too_many_arguments)]
#[cold]
pub(super) fn try_draw<P: Painter>(
    context: &P,
    text: &str,
    x: f64,
    y: f64,
    style: &TextStyle,
    fonts: &[Rc<str>],
    glyphs: &[GlyphRunStep],
    font_state: &mut FontStateTracker,
    dpi: f64,
) -> Result<bool, P::Error> {
    if eligible(context, text, style) && fonts.len() == 1 && glyphs.len() == text.len() {
        // Keep the tracked Calibri advance model ahead of font-size-only drawing.
        let tracked_calibri = style.letter_spacing_px != 0.0
            && style.enable_pair_kerning
            && regular_calibri(&fonts[0]).is_some();
        if let Some((prefix, suffix, realized)) =
            realized_print_parts(&fonts[0], dpi).filter(|_| !tracked_calibri)
        {
            if glyphs.iter().enumerate().all(|(index, glyph)| {
                glyph.font_index == 0
                    && glyph.text_start as usize == index
                    && glyph.text_end as usize == index + 1
                    && glyph.x_offset.is_finite()
            }) {
                let font: Rc<str> = Rc::from(format!("{prefix}{realized:.6}px{suffix}"));
                font_state.set_font_rc_if_changed(context, &font);
                for glyph in glyphs {
                    context.fill_text(
                        glyph.text(text).unwrap_or_default(),
                        x + glyph.x_offset,
                        y,
                    )?;
                }
                return Ok(true);
            }
        }
    }
    if !eligible(context, text, style)
        || fonts.len() != 1
        || glyphs.len() != text.len()
        || !dpi.is_finite()
        || dpi <= 0.0
    {
        return Ok(false);
    }
    let Some((prefix, suffix)) = regular_calibri(&fonts[0]) else {
        return Ok(false);
    };
    let Some(source_em) = jade_docx_layout::line_metrics::font_em_px(&fonts[0]) else {
        return Ok(false);
    };
    let requested_pt = f64::from(style.font_size_pt);
    let print_em = (requested_pt * dpi / 72.0).round() * 96.0 / dpi;
    if !print_em.is_finite() || print_em <= 0.0 {
        return Ok(false);
    }
    if glyphs.iter().enumerate().any(|(index, glyph)| {
        glyph.font_index != 0
            || glyph.text_start as usize != index
            || glyph.text_end as usize != index + 1
            || !glyph.x_offset.is_finite()
    }) {
        return Ok(false);
    }
    let Some(first) = glyphs.first() else {
        return Ok(false);
    };
    font_state.set_font_rc_if_changed(context, &fonts[0]);
    // Font-size realization keeps authored tracking in logical units. Only
    // normalize integer source-device advances when its em already matches
    // the physical em; otherwise retain the established scaled advance path.
    if print_em_matches_source(source_em, print_em) {
        if let Some(widths) = context.measure_windows_pdf_ascii_widths(text)? {
            if let Some(origins) = normalized_print_origins(glyphs, &widths, print_em, dpi) {
                let font: Rc<str> = Rc::from(format!("{prefix}{print_em:.6}px{suffix}"));
                font_state.set_font_rc_if_changed(context, &font);
                for (glyph, origin) in glyphs.iter().zip(origins) {
                    context.fill_text(glyph.text(text).unwrap_or_default(), x + origin, y)?;
                }
                return Ok(true);
            }
        }
        return Ok(false);
    }
    let ratio = print_em / source_em;
    let font: Rc<str> = Rc::from(format!("{prefix}{print_em:.6}px{suffix}"));
    font_state.set_font_rc_if_changed(context, &font);
    for (index, glyph) in glyphs.iter().enumerate() {
        let tracking = index as f64 * style.letter_spacing_px;
        let relative = glyph.x_offset - first.x_offset;
        let print_x = first.x_offset + (relative - tracking) * ratio + tracking;
        context.fill_text(glyph.text(text).unwrap_or_default(), x + print_x, y)?;
    }
    Ok(true)
}

fn print_em_matches_source(source: f64, physical: f64) -> bool {
    source.is_finite()
        && physical.is_finite()
        && source > 0.0
        && physical > 0.0
        && (source - physical).abs() < 1e-9
}

// Word's physical text stream normalizes integer device advances by the
// realized font em. Obtain widths from the exact selected source font in GDI.
// Logical glyph origins, screen pixels, tracking and line breaking are untouched.
fn normalized_print_origins(
    glyphs: &[GlyphRunStep],
    widths: &[f64],
    em: f64,
    dpi: f64,
) -> Option<Vec<f64>> {
    if glyphs.len() != widths.len()
        || glyphs.is_empty()
        || !em.is_finite()
        || em <= 0.0
        || !dpi.is_finite()
        || dpi <= 0.0
        || widths.iter().any(|w| !w.is_finite() || *w <= 0.0)
    {
        return None;
    }
    let height = (em * dpi / 96.0).round();
    if height <= 0.0 || !height.is_finite() {
        return None;
    }
    let first = glyphs[0].x_offset;
    let phases: Vec<f64> = glyphs
        .iter()
        .map(|g| ((g.x_offset - first) * dpi / 96.0).round())
        .collect();
    if phases.iter().any(|x| !x.is_finite()) || phases.windows(2).any(|p| p[1] < p[0]) {
        return None;
    }
    let mut x = first;
    let mut debt = 0.0;
    let mut origins = Vec::with_capacity(glyphs.len());
    for (i, width) in widths.iter().enumerate() {
        origins.push(x);
        if i + 1 == widths.len() {
            break;
        }
        let dx = phases[i + 1] - phases[i];
        debt += width - dx * f64::from((1.0 / height) as f32) * 1000.0;
        let adjustment = debt.trunc();
        if adjustment != 0.0 {
            debt = 0.0;
        }
        x += (width - adjustment) * em / 1000.0;
    }
    origins.iter().all(|x| x.is_finite()).then_some(origins)
}

#[cfg(test)]
mod tests {
    use super::{normalized_print_origins, regular_calibri, GlyphRunStep};

    #[test]
    fn physical_em_realization_handles_body_fonts_and_styles() {
        for (descriptor, expected) in [
            ("10.5pt Arial", 14.08),
            ("bold 16pt Calibri", 21.28),
            ("italic 13pt Arial", 17.28),
        ] {
            let (_, _, em) = super::realized_print_parts(descriptor, 600.0).unwrap();
            assert!((em - expected).abs() < 1e-9);
        }
        assert!(super::realized_print_parts("12pt Arial", 600.0).is_none());
        assert!(super::realized_print_parts("NaNpt Arial", 600.0).is_none());
        assert!(super::realized_print_parts("12pt Arial", f64::NAN).is_none());
        assert!(super::realized_print_parts("12pt Arial", 0.0).is_none());
    }

    #[test]
    fn resized_print_fonts_keep_logical_tracking_on_the_scaled_path() {
        assert!(super::print_em_matches_source(32.0, 32.0));
        assert!(!super::print_em_matches_source(
            22.0 * 96.0 / 72.0,
            183.0 * 96.0 / 600.0
        ));
        for (source, physical) in [(0.0, 0.0), (f64::NAN, 32.0), (32.0, f64::NAN)] {
            assert!(!super::print_em_matches_source(source, physical));
        }
    }

    fn glyphs(offsets: &[f64]) -> Vec<GlyphRunStep> {
        offsets
            .iter()
            .enumerate()
            .map(|(i, x)| GlyphRunStep {
                text_start: i as u32,
                text_end: i as u32 + 1,
                font_index: 0,
                x_offset: *x,
            })
            .collect()
    }

    #[test]
    fn realized_em_retains_origin_and_integer_device_advance_at_both_sizes() {
        let glyphs = glyphs(&[3.25, 16.083333, 35.8]);
        for em in [29.28, 32.0] {
            let origins =
                normalized_print_origins(&glyphs, &[579.0, 488.0, 300.0], em, 600.0).unwrap();
            assert_eq!(origins[0], 3.25);
            for (actual, source) in origins.iter().zip(&glyphs) {
                let device = ((source.x_offset - 3.25) * 600.0 / 96.0).round();
                assert!((actual - 3.25 - device * 96.0 / 600.0).abs() < 0.1);
            }
        }
    }

    #[test]
    fn invalid_or_reversed_print_metrics_keep_the_existing_path() {
        let good = glyphs(&[0.0, 10.0]);
        for widths in [
            vec![],
            vec![500.0],
            vec![500.0, f64::NAN],
            vec![500.0, -1.0],
        ] {
            assert!(normalized_print_origins(&good, &widths, 32.0, 600.0).is_none());
        }
        assert!(normalized_print_origins(&good, &[500.0, 500.0], 0.0, 600.0).is_none());
        assert!(normalized_print_origins(&good, &[500.0, 500.0], 32.0, f64::NAN).is_none());
        assert!(
            normalized_print_origins(&glyphs(&[10.0, 0.0]), &[500.0, 500.0], 32.0, 600.0).is_none()
        );
        assert!(
            normalized_print_origins(&glyphs(&[0.0, f64::NAN]), &[500.0, 500.0], 32.0, 600.0)
                .is_none()
        );
    }

    #[test]
    fn measured_face_guard_keeps_other_fonts_and_styles_on_their_paths() {
        assert!(regular_calibri("22.00pt Calibri, serif").is_some());
        assert!(regular_calibri("400 29.33px 'Calibri', 'Noto Sans JP', serif").is_some());
        for font in [
            "bold 22pt Calibri",
            "italic 22pt Calibri",
            "22pt Arial, Calibri",
            "22pt 'Calibri Light'",
            "NaNpt Calibri",
            "0px Calibri",
        ] {
            assert!(regular_calibri(font).is_none(), "{font}");
        }
    }
}
