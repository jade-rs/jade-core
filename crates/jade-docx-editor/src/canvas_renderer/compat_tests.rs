//! Legacy layout options of `w:compat` the renderer honours.

use super::recording_host::recording_renderer;
use super::*;
use jade_docx_reader::settings::CompatOptions;
use jade_painter::recording::Op;

fn run(text: &str, properties: Option<RunProperties>) -> Run {
    Run {
        text: text.into(),
        properties,
        image: None,
        text_box: None,
        math: None,
        field: None,
        hyperlink: None,
        node_id: None,
        text_node_id: None,
        revision_node_id: None,
        comment_reference_ids: Vec::new(),
        footnote_reference_ids: Vec::new(),
        endnote_reference_ids: Vec::new(),
        note_separator: None,
        has_footnote_ref: false,
        has_endnote_ref: false,
        revision: None,
    }
}

fn document(paragraph: Paragraph, options: &[&str]) -> Document {
    Document {
        body: Some(Body {
            elements: vec![BodyElement::from_paragraph(paragraph)],
        }),
        compat: CompatOptions {
            settings: Vec::new(),
            options: options
                .iter()
                .map(|name| (name.to_string(), true))
                .collect(),
        },
        ..Document::default()
    }
}

fn ops(document: &Document) -> Vec<Op> {
    let mut renderer = recording_renderer();
    renderer.render(document, &Options::default()).unwrap();
    renderer.host.painter().inner().ops()
}

/// x of the first glyph drawn as `text`.
fn glyph_x(ops: &[Op], text: &str) -> f64 {
    ops.iter()
        .find_map(|op| match op {
            Op::FillText(value, x, _) if value == text => Some(*x),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {text}"))
}

#[test]
fn a_justified_line_ending_in_a_manual_break_is_spread_unless_the_document_says_not_to() {
    let paragraph = Paragraph {
        runs: vec![run("aa bb\ncc", None)],
        properties: Some(ParagraphProperties {
            alignment: Some("both".into()),
            ..ParagraphProperties::default()
        }),
        node_id: None,
        comment_ranges: Vec::new(),
        comment_markers: Vec::new(),
    };
    let spread = glyph_x(&ops(&document(paragraph.clone(), &[])), "b");
    let kept = glyph_x(&ops(&document(paragraph, &["doNotExpandShiftReturn"])), "b");
    assert!(spread > kept + 100.0, "spread {spread} vs kept {kept}");
}

/// The right end of the underline under `text`'s line.
fn underline_end(ops: &[Op]) -> f64 {
    let mut stroking = false;
    let mut end: f64 = 0.0;
    let mut points = Vec::new();
    for op in ops {
        match op {
            Op::SetStrokeStyle(color) => stroking = color.eq_ignore_ascii_case("#ff0000"),
            Op::MoveTo(x, _) | Op::LineTo(x, _) if stroking => points.push(*x),
            Op::Stroke if stroking => {
                end = points.iter().copied().fold(end, f64::max);
                points.clear();
            }
            _ => {}
        }
    }
    end
}

#[test]
fn trailing_spaces_are_underlined_only_under_ul_trail_space() {
    let underlined = RunProperties {
        underline: Some(true),
        underline_color: Some("#ff0000".into()),
        ..RunProperties::default()
    };
    let paragraph = Paragraph {
        runs: vec![run("ab   ", Some(underlined))],
        properties: None,
        node_id: None,
        comment_ranges: Vec::new(),
        comment_markers: Vec::new(),
    };
    let start = glyph_x(&ops(&document(paragraph.clone(), &[])), "a");
    let without = underline_end(&ops(&document(paragraph.clone(), &[]))) - start;
    let with = underline_end(&ops(&document(paragraph, &["ulTrailSpace"]))) - start;
    assert!(
        (without - 20.0).abs() < 0.5,
        "only the ink is underlined: {without}"
    );
    assert!((with - 50.0).abs() < 0.5, "the spaces are too: {with}");
}
