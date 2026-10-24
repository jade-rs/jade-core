//! Word shapes drawn from ECMA-376's preset definitions: the presets outside
//! the hand-traced set, and any preset whose adjust handles moved.

use super::*;
use jade_docx_reader::document::{PresetGeometry, PresetGeometryId};
use jade_painter::recording::{Op, RecordingPainter};

fn preset(name: &str, adjustments: &[(&str, i64)]) -> TextBoxShape {
    TextBoxShape::Preset(PresetGeometryId::intern(PresetGeometry {
        name: name.into(),
        adjustments: adjustments
            .iter()
            .map(|(name, value)| (name.to_string(), *value))
            .collect(),
    }))
}

fn traced(shape: TextBoxShape) -> Vec<(f64, f64)> {
    let painter = RecordingPainter::new();
    <CanvasRenderer>::trace_text_box_shape(&painter, shape, 10.0, 20.0, 100.0, 50.0).unwrap();
    painter
        .ops()
        .into_iter()
        .filter_map(|op| match op {
            Op::MoveTo(x, y) | Op::LineTo(x, y) => Some((x, y)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_preset_outside_the_traced_set_draws_its_own_outline() {
    let heptagon = traced(preset("heptagon", &[]));
    assert_eq!(heptagon.len(), 7, "{heptagon:?}");
    // The definition scales its vertices a little past the box (`hf`, `vf`).
    for &(x, y) in &heptagon {
        assert!(
            (5.0..=115.0).contains(&x) && (15.0..=75.0).contains(&y),
            "{x},{y}"
        );
    }
    // Every one of the presets Word has no hand-traced outline for is
    // drawn as itself rather than as a rectangle.
    for name in [
        "gear6",
        "mathDivide",
        "curvedUpArrow",
        "horizontalScroll",
        "funnel",
    ] {
        assert!(traced(preset(name, &[])).len() > 4, "{name}");
    }
}

#[test]
fn moved_adjust_handles_change_the_outline() {
    let default = traced(preset("rightArrow", &[]));
    let thin = traced(preset("rightArrow", &[("adj1", 20_000)]));
    assert_ne!(default, thin);
    // adj1 is the shaft's share of the height: 20% leaves it 10px thick.
    let shaft: Vec<f64> = thin.iter().map(|&(_, y)| y).collect();
    assert!(shaft.iter().any(|y| (y - 40.0).abs() < 1e-6), "{shaft:?}");
    assert!(shaft.iter().any(|y| (y - 50.0).abs() < 1e-6), "{shaft:?}");
}

#[test]
fn an_unknown_preset_falls_back_to_its_box() {
    let painter = RecordingPainter::new();
    <CanvasRenderer>::trace_text_box_shape(
        &painter,
        preset("notAPreset", &[]),
        0.0,
        0.0,
        10.0,
        10.0,
    )
    .unwrap();
    assert!(painter.ops().contains(&Op::Rect(0.0, 0.0, 10.0, 10.0)));
}
