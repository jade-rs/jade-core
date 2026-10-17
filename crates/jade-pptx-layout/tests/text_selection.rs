use jade_pptx_layout::{layout_slide, HeuristicMeasure, RunStyle, TextMeasure};
use jade_pptx_reader::{parse_slide_xml, SlideSize, ThemeColors};
use std::cell::Cell;

fn shape(body: &str, width: usize) -> jade_pptx_layout::PlacedShape {
    let xml = format!(
        r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{}" cy="1905000"/></a:xfrm></p:spPr><p:txBody><a:bodyPr lIns="0" rIns="0" tIns="0" bIns="0"/>{body}</p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#,
        width * 9525
    );
    layout_slide(
        SlideSize::default(),
        &parse_slide_xml(&xml).unwrap(),
        &ThemeColors::default(),
        &HeuristicMeasure,
    )
    .shapes
    .remove(0)
}
fn paragraph(text: &str) -> String {
    format!("<a:p><a:r><a:rPr sz=\"1200\"/><a:t>{text}</a:t></a:r></a:p>")
}

#[test]
fn source_offsets_cover_wraps_empty_paragraphs_breaks_and_surrogates() {
    let s = shape(&(paragraph("日🦀本") + "<a:p/>" + &paragraph("a\n\nb")), 30);
    let offsets: Vec<_> = s
        .lines
        .iter()
        .flat_map(|line| &line.runs)
        .flat_map(|r| r.source_offsets.clone())
        .collect();
    assert!(
        offsets.contains(&0)
            && offsets.contains(&1)
            && offsets.contains(&3)
            && offsets.contains(&4)
    );
    assert!(
        !offsets.contains(&2),
        "a surrogate pair is one grapheme boundary"
    );
    assert!(s.lines.iter().any(|line| line.source_range == Some((5, 5))));
    assert!(s.lines.iter().any(|line| line.source_range == Some((8, 8))));
    for offset in [0, 1, 3, 4, 5, 6, 7, 8, 9, 10] {
        assert!(s
            .selection_geometry(offset, offset, offset, false, &HeuristicMeasure)
            .is_some());
    }
}

#[test]
fn capitals_expansion_and_cross_run_graphemes_keep_original_offsets() {
    let s = shape(
        "<a:p><a:r><a:rPr cap=\"all\"/><a:t>ßa</a:t></a:r></a:p>",
        400,
    );
    assert_eq!(s.lines[0].runs[0].text, "SSA");
    assert_eq!(s.lines[0].runs[0].source_offsets, vec![0, 0, 1, 2]);
    let s = shape(
        "<a:p><a:r><a:t>👩</a:t></a:r><a:r><a:t>‍💻</a:t></a:r></a:p>",
        400,
    );
    let text: String = s.lines[0]
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect();
    assert_eq!(text, "👩‍💻");
    assert_eq!(s.lines[0].runs[0].source_offsets, vec![0, 5]);
    let s = shape(
        "<a:p><a:r><a:rPr sz=\"1200\" cap=\"all\"/><a:t>ß</a:t></a:r></a:p>",
        8,
    );
    assert_eq!(s.lines.len(), 2);
    let before = s
        .selection_geometry(0, 0, 0, false, &HeuristicMeasure)
        .unwrap();
    let after = s
        .selection_geometry(0, 1, 1, false, &HeuristicMeasure)
        .unwrap();
    assert!(before.caret[0].y < after.caret[0].y);
    assert_eq!(after.highlights.len(), 2);
    assert!(after.highlights.iter().all(|quad| quad[1].x > quad[0].x));
}

#[test]
fn bullets_do_not_acquire_editable_character_positions() {
    let s = shape(
        "<a:p><a:pPr><a:buChar char=\"•\"/></a:pPr><a:r><a:t>text</a:t></a:r></a:p>",
        400,
    );
    assert!(s.lines[0].runs[0].source_offsets.is_empty());
    let caret = s
        .selection_geometry(0, 0, 0, false, &HeuristicMeasure)
        .unwrap();
    assert_eq!(caret.caret[0].x, s.lines[0].runs[1].x);
    assert!(caret.highlights.is_empty());
}

#[test]
fn caret_measurement_retains_following_kerning_context() {
    struct Kerned;
    impl TextMeasure for Kerned {
        fn measure(&self, text: &str, _: &RunStyle) -> f64 {
            text.len() as f64 * 10.0 - if text.contains("AV") { 2.0 } else { 0.0 }
        }
    }
    let mut s = shape(&paragraph("AVX"), 400);
    s.lines[0].runs[0].width = 28.0;
    let geometry = s.selection_geometry(1, 1, 1, false, &Kerned).unwrap();
    assert_eq!(geometry.caret[0].x, 8.0);
    assert_eq!(s.hit_text(8.0, 5.0, &Kerned).unwrap().offset, 1);
}

#[test]
fn selecting_empty_paragraphs_hard_breaks_and_elided_wrap_spaces_is_visible() {
    let s = shape(&(paragraph("a") + "<a:p/>" + &paragraph("b\n\nc")), 400);
    let selection = s
        .selection_geometry(1, 6, 6, false, &HeuristicMeasure)
        .unwrap();
    assert_eq!(selection.highlights.len(), 4);
    assert!(selection
        .highlights
        .iter()
        .all(|quad| quad[1].x > quad[0].x));
    let s = shape(&paragraph("word word"), 40);
    assert_eq!(s.lines[0].selection_tail, Some((4, 5)));
    let selection = s
        .selection_geometry(4, 5, 5, false, &HeuristicMeasure)
        .unwrap();
    assert_eq!(selection.highlights.len(), 1);
    assert!(selection.highlights[0][1].x > selection.highlights[0][0].x);
}

#[test]
fn hit_testing_and_geometry_round_trip_rotated_flipped_and_vertical_frames() {
    let mut s = shape(&paragraph("abcd"), 400);
    for (rotation, text_rotation, flip_h, flip_v) in [
        (0.0, 0.0, false, false),
        (0.7, 0.0, true, false),
        (1.5, 90.0, false, true),
        (0.0, 270.0, true, true),
    ] {
        s.rotation_rad = rotation;
        s.text_rotation_deg = text_rotation;
        s.flip_h = flip_h;
        s.flip_v = flip_v;
        let geometry = s
            .selection_geometry(0, 3, 3, false, &HeuristicMeasure)
            .unwrap();
        let p = geometry.caret;
        let hit = s
            .hit_text(
                (p[0].x + p[1].x) / 2.0,
                (p[0].y + p[1].y) / 2.0,
                &HeuristicMeasure,
            )
            .unwrap();
        assert_eq!(hit.offset, 3);
        assert_eq!(geometry.highlights.len(), 1);
    }
}

#[test]
fn soft_wrap_boundaries_have_upstream_and_downstream_carets() {
    let s = shape(&paragraph("日本"), 17);
    assert_eq!(s.lines.len(), 2);
    let a = s
        .selection_geometry(1, 1, 1, true, &HeuristicMeasure)
        .unwrap();
    let b = s
        .selection_geometry(1, 1, 1, false, &HeuristicMeasure)
        .unwrap();
    assert!(a.caret[0].y < b.caret[0].y);
    assert_eq!(
        s.move_text_caret(0, "line_end", false, &HeuristicMeasure)
            .unwrap()
            .offset,
        1
    );
}

#[test]
fn an_unwrapped_long_line_uses_logarithmically_many_measurements_for_a_hit() {
    let mut s = shape(&paragraph("x"), 400);
    let run = &mut s.lines[0].runs[0];
    run.text = "x".repeat(16384);
    run.source_offsets = (0..=16384).collect();
    run.width = 16384.0;
    s.lines[0].source_range = Some((0, 16384));
    struct Count(Cell<usize>);
    impl TextMeasure for Count {
        fn measure(&self, text: &str, _: &RunStyle) -> f64 {
            self.0.set(self.0.get() + 1);
            text.len() as f64
        }
    }
    let measure = Count(Cell::new(0));
    assert_eq!(s.hit_text(8192.0, 5.0, &measure).unwrap().offset, 8192);
    assert!(measure.0.get() <= 40, "{} measurements", measure.0.get());
}

#[test]
fn repeated_vertical_navigation_retains_the_column_across_short_and_empty_lines() {
    let mut s = shape(
        &(paragraph("abcdefghij") + &paragraph("x") + "<a:p/>" + &paragraph("abcdefghij")),
        400,
    );
    for (rotation, text_rotation, flip_h, flip_v) in [
        (0.0, 0.0, false, false),
        (0.7, 0.0, true, false),
        (1.5, 90.0, false, true),
        (0.0, 270.0, true, true),
    ] {
        s.rotation_rad = rotation;
        s.text_rotation_deg = text_rotation;
        s.flip_h = flip_h;
        s.flip_v = flip_v;
        let mut offset = 8;
        let mut upstream = false;
        let mut preferred = None;
        for (direction, expected) in [
            ("up", 8),
            ("down", 12),
            ("down", 13),
            ("down", 22),
            ("down", 22),
            ("up", 13),
            ("up", 12),
            ("up", 8),
        ] {
            let movement = s
                .move_text_caret_with_x(offset, direction, upstream, preferred, &HeuristicMeasure)
                .unwrap();
            assert_eq!(movement.hit.offset, expected, "{direction} at {offset}");
            if let Some(x) = preferred {
                assert_eq!(movement.preferred_x, Some(x));
            }
            offset = movement.hit.offset;
            upstream = movement.hit.upstream;
            preferred = movement.preferred_x;
        }
    }
}

#[test]
fn vertical_navigation_uses_text_frame_advances_for_aligned_lines_and_resets_at_line_edges() {
    let s = shape(
        &(paragraph("abcdefghij")
            + "<a:p><a:pPr algn=\"ctr\"/><a:r><a:t>x</a:t></a:r></a:p>"
            + &paragraph("abcdefghij")),
        400,
    );
    let first = s
        .move_text_caret_with_x(8, "down", false, None, &HeuristicMeasure)
        .unwrap();
    assert_eq!(first.hit.offset, 11);
    let next = s
        .move_text_caret_with_x(
            first.hit.offset,
            "down",
            first.hit.upstream,
            first.preferred_x,
            &HeuristicMeasure,
        )
        .unwrap();
    assert_eq!(next.hit.offset, 21);
    for direction in ["line_start", "line_end"] {
        let edge = s
            .move_text_caret_with_x(21, direction, false, next.preferred_x, &HeuristicMeasure)
            .unwrap();
        assert_eq!(edge.preferred_x, None);
    }
    for x in [f64::INFINITY, f64::NAN, f64::NEG_INFINITY] {
        assert!(s
            .move_text_caret_with_x(8, "down", false, Some(x), &HeuristicMeasure)
            .is_none());
    }
}

#[test]
fn retained_navigation_positions_remain_utf16_grapheme_boundaries() {
    let text = "A🦀é👩‍💻日本";
    let s = shape(&(paragraph(text) + "<a:p/>" + &paragraph(text)), 400);
    let first = s
        .move_text_caret_with_x(10, "down", false, None, &HeuristicMeasure)
        .unwrap();
    let next = s
        .move_text_caret_with_x(
            first.hit.offset,
            "down",
            first.hit.upstream,
            first.preferred_x,
            &HeuristicMeasure,
        )
        .unwrap();
    assert_eq!(next.hit.offset, text.encode_utf16().count() + 2 + 10);
    assert!(s.lines[2].runs[0].source_offsets.contains(&next.hit.offset));
}
