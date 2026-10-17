//! Inspect a host's chosen static math font without registering or shipping it.
use jade_docx_layout::math_layout::font::{MathStretchAxis, MathStretchPlan, OpenTypeMathFont};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: math_font_info FONT [FACE_INDEX]")?;
    let index: u32 = args.next().map_or(Ok(0), |v| v.parse())?;
    let bytes = std::fs::read(&path)?;
    let f = OpenTypeMathFont::parse(&bytes, index)
        .map_err(|e| format!("cannot read math font: {e:?}"))?;
    let c = f.constants();
    let base = f.glyph_id('∫');
    let display = base.and_then(|g| {
        match f.stretch_plan(
            g,
            MathStretchAxis::Vertical,
            u32::from(c.display_operator_min_height),
        ) {
            Some(MathStretchPlan::Glyph(v)) => Some(v),
            _ => None,
        }
    });
    let display_geometry = display.map(|v| -> Result<_, String> {
        let g = f.glyph_geometry(v.glyph_id, 8.0 * 96.0 / 72.0).ok_or("selected variant has no geometry")?;
        let outline = f.glyph_outline(v.glyph_id, 16384).map_err(|e| format!("outline: {e:?}"))?.ok_or("selected variant has no outline")?;
        Ok(json!({"glyph_id": v.glyph_id, "extension_units": v.extension_units,
            "advance_px_at_8pt": g.advance_px,
            "ink_units": [outline.bounds.x_min, outline.bounds.y_min, outline.bounds.x_max, outline.bounds.y_max],
            "outline_commands": outline.commands.len(),
            "italic_correction_units": f.italic_correction(v.glyph_id),
            "extended_shape": f.is_extended_shape(v.glyph_id)}))
    }).transpose()?;
    let script_glyphs = ['𝑎', '𝑏', 'x'].into_iter().filter_map(|character| {
                let base = f.glyph_id(character)?;
                Some(json!({"character": character.to_string(), "root": base,
                    "script": f.script_glyph_id(base, 1), "script_script": f.script_glyph_id(base, 2)}))
            }).collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "path": path, "face_index": index, "units_per_em": f.units_per_em(),
            "constants": {"script_percent": c.script_percent, "script_script_percent": c.script_script_percent,
                "delimited_min_height": c.delimited_min_height, "display_operator_min_height": c.display_operator_min_height,
                "leading": c.leading, "axis_height": c.axis_height,
                "superscript_shift_up": c.superscript_shift_up, "subscript_shift_down": c.subscript_shift_down,
                "superscript_baseline_drop_max": c.superscript_baseline_drop_max, "subscript_baseline_drop_min": c.subscript_baseline_drop_min,
                "upper_limit_gap_min": c.upper_limit_gap_min, "upper_limit_baseline_rise_min": c.upper_limit_baseline_rise_min,
                "lower_limit_gap_min": c.lower_limit_gap_min, "lower_limit_baseline_drop_min": c.lower_limit_baseline_drop_min,
                "space_after_script": c.space_after_script},
            "integral_base_glyph_id": base, "integral_display": display_geometry,
            "script_glyphs": script_glyphs
        }))?
    );
    Ok(())
}
