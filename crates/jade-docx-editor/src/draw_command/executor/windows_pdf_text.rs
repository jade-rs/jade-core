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
            || realized_aptos_parts(&style.font_descriptor_latin_ascii, dpi).is_some())
}

// Actual Windows Word 16.0.19127.20936 PDFs independently retain this
// realization on all 56 long-run controls (four styles, seven point sizes).
// Only the physical font changes: the precomputed logical origins stay intact.
fn realized_aptos_parts(descriptor: &str, dpi: f64) -> Option<(&str, &str, f64)> {
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
        if !family.eq_ignore_ascii_case("Aptos Display") {
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
        if let Some((prefix, suffix, realized)) = realized_aptos_parts(&fonts[0], dpi) {
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
    if !print_em.is_finite() || print_em <= 0.0 || (source_em - print_em).abs() < 1e-9 {
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

#[cfg(test)]
mod tests {
    use super::regular_calibri;

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
