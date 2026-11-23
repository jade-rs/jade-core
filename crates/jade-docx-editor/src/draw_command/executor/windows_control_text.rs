use super::{windows_pdf_text, FontStateTracker, TextStyle};
use crate::draw_command::GlyphRunStep;
use jade_painter::Painter;
use std::rc::Rc;

fn plain_arial(descriptor: &str) -> bool {
    let descriptor = jade_painter::font_features::strip(descriptor);
    for token in descriptor.split_whitespace() {
        let Some(number) = token
            .strip_suffix("pt")
            .or_else(|| token.strip_suffix("px"))
        else {
            continue;
        };
        let Ok(size) = number.parse::<f64>() else {
            return false;
        };
        if !size.is_finite() || size <= 0.0 {
            return false;
        }
        let Some(at) = descriptor.find(token) else {
            return false;
        };
        let prefix = descriptor[..at].trim();
        let family = descriptor[at + token.len()..]
            .split(',')
            .next()
            .unwrap_or_default()
            .trim()
            .trim_matches(['\'', '"']);
        return (prefix.is_empty() || prefix.eq_ignore_ascii_case("normal"))
            && (family.eq_ignore_ascii_case("Arial") || family.eq_ignore_ascii_case("ArialMT"));
    }
    false
}

fn nonjoining_features(descriptor: &str) -> bool {
    if !jade_painter::font_features::has_features(descriptor) {
        return true;
    }
    let features = jade_painter::font_features::parse(descriptor);
    !features.is_empty()
        && features
            .iter()
            .all(|f| f.value == 0 || (f.tag == *b"kern" && f.value == 1))
}

fn plain_windows_style(text: &str, style: &TextStyle) -> bool {
    jade_docx_layout::line_metrics::word_layout_platform()
        == jade_docx_layout::line_metrics::WordLayoutPlatform::Windows
        && text.contains(['\u{200b}', '\u{200c}', '\u{200d}'])
        && (style.text_scale_x - 1.0).abs() <= f64::EPSILON
        && !style.outline
        && !style.shadow
        && !style.emboss
        && !style.imprint
        && style.effects.is_none()
        && plain_arial(&style.font_descriptor_latin_ascii)
        && nonjoining_features(&style.font_descriptor_latin_ascii)
}

// The unshaped replay path also receives isolated controls when a captured
// font subset has no cmap entry. Keep its original measured advance while
// omitting only the nonprinting payload from the painter.
pub(super) fn try_draw_simple<P: Painter>(
    context: &P,
    text: &str,
    x: f64,
    y: f64,
    style: &TextStyle,
    font_state: &mut FontStateTracker,
) -> Result<Option<f64>, P::Error> {
    if !plain_windows_style(text, style) || style.letter_spacing_px != 0.0 {
        return Ok(None);
    }
    let Some(visible) = visible_piece(text) else {
        return Ok(None);
    };
    if visible == text {
        return Ok(None);
    }
    let logical = style.font_for_grapheme(text, super::classify_script(text));
    if !nonjoining_features(logical) || (!visible.is_empty() && !plain_arial(logical)) {
        return Ok(None);
    }
    font_state.set_fill_style_rc_if_changed(context, &style.fill_color);
    font_state.set_font_if_changed(context, logical);
    let width = font_state.measure_text_cached(context, text, logical)?;
    if !visible.is_empty() {
        let physical = windows_pdf_text::single_scalar_font(context, visible, style);
        if let Some(font) = physical.as_ref() {
            font_state.set_font_rc_if_changed(context, font);
        }
        context.fill_text(visible, x, y)?;
    }
    Ok(Some(width))
}

fn visible_piece(text: &str) -> Option<&str> {
    if !text.is_empty()
        && text
            .chars()
            .all(|c| matches!(c, '\u{200b}' | '\u{200c}' | '\u{200d}'))
    {
        return Some("");
    }
    let mut chars = text.chars();
    let first = chars.next()?;
    if !(' '..='~').contains(&first) {
        return None;
    }
    if chars.clone().next().is_none() {
        return Some(text);
    }
    (first.is_ascii_alphabetic() && chars.all(|c| matches!(c, '\u{200c}' | '\u{200d}')))
        .then_some(&text[..first.len_utf8()])
}

// Source ranges and all precomputed origins remain intact. Only the painted
// payload omits measured Word nonprinting controls, so an original PDF font
// subset without their cmap entries cannot paint .notdef boxes.
#[allow(clippy::too_many_arguments)]
pub(super) fn try_draw<P: Painter>(
    context: &P,
    text: &str,
    x: f64,
    y: f64,
    style: &TextStyle,
    fonts: &[Rc<str>],
    glyphs: &[GlyphRunStep],
    font_state: &mut FontStateTracker,
) -> Result<bool, P::Error> {
    if !plain_windows_style(text, style)
        || fonts.is_empty()
        || fonts.iter().any(|font| !nonjoining_features(font))
    {
        return Ok(false);
    }
    let mut cursor = 0;
    let mut changed = false;
    for glyph in glyphs {
        let start = glyph.text_start as usize;
        let end = glyph.text_end as usize;
        if start != cursor
            || end <= start
            || !glyph.x_offset.is_finite()
            || glyph.font_index as usize >= fonts.len()
        {
            return Ok(false);
        }
        let Some(part) = text.get(start..end) else {
            return Ok(false);
        };
        let Some(visible) = visible_piece(part) else {
            return Ok(false);
        };
        if !visible.is_empty() && !plain_arial(&fonts[glyph.font_index as usize]) {
            return Ok(false);
        }
        changed |= visible != part;
        cursor = end;
    }
    if cursor != text.len() || !changed {
        return Ok(false);
    }
    for glyph in glyphs {
        let part = glyph.text(text).expect("validated complete UTF8 range");
        let visible = visible_piece(part).expect("validated source piece");
        if visible.is_empty() {
            continue;
        }
        let physical = windows_pdf_text::single_scalar_font(context, visible, style);
        let font = physical
            .as_ref()
            .unwrap_or(&fonts[glyph.font_index as usize]);
        font_state.set_font_rc_if_changed(context, font);
        context.fill_text(visible, x + glyph.x_offset, y)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_measured_latin_controls_project_to_a_painted_letter() {
        assert_eq!(super::visible_piece("A\u{200c}\u{200d}"), Some("A"));
        assert_eq!(super::visible_piece("\u{200b}\u{200d}"), Some(""));
        for text in [
            "ا\u{200d}",
            "क\u{200d}",
            "A\u{301}\u{200d}",
            "fi\u{200d}",
            "1\u{200d}",
        ] {
            assert!(super::visible_piece(text).is_none());
        }
    }
}
