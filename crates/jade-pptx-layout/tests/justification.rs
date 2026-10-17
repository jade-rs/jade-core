use jade_pptx_layout::{layout_slide, HeuristicMeasure, PlacedShape};
use jade_pptx_reader::{parse_slide_xml, SlideSize, ThemeColors};
fn shape(body: &str, width: usize) -> PlacedShape {
    let xml = format!(
        r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="2" name="Text"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{}" cy="4000500"/></a:xfrm></p:spPr><p:txBody><a:bodyPr lIns="0" rIns="0" tIns="0" bIns="0"/>{body}</p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#,
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
fn paragraph(alignment: &str, text: &str) -> String {
    format!(
        "<a:p><a:pPr algn=\"{alignment}\"/><a:r><a:rPr sz=\"1200\"/><a:t>{text}</a:t></a:r></a:p>"
    )
}
fn right(s: &PlacedShape, line: usize) -> f64 {
    let r = s.lines[line].runs.last().unwrap();
    r.x + r.width
}
fn round_trip(s: &PlacedShape) {
    let mut boundaries = std::collections::BTreeSet::new();
    for line in &s.lines {
        for run in &line.runs {
            boundaries.extend(&run.source_offsets);
        }
    }
    for offset in boundaries {
        let g = s
            .selection_geometry(offset, offset, offset, false, &HeuristicMeasure)
            .unwrap();
        let c = &g.caret[0];
        let hit = s
            .hit_text(
                (c.x + g.caret[1].x) / 2.0,
                (c.y + g.caret[1].y) / 2.0,
                &HeuristicMeasure,
            )
            .unwrap();
        assert_eq!(hit.offset, offset, "caret {offset} at {c:?}");
    }
}
#[test]
fn latin_wraps_expand_separators_preserving_words_and_last_line() {
    let s = shape(&paragraph("just", "one two three"), 90);
    assert_eq!(s.lines.len(), 2);
    assert_eq!(
        s.lines[0]
            .runs
            .iter()
            .map(|r| r.text.as_str())
            .collect::<String>(),
        "one two"
    );
    assert_eq!(s.lines[0].runs[0].text, "one ");
    assert_eq!(s.lines[0].runs[1].text, "two");
    assert!((right(&s, 0) - 90.0).abs() < 1e-9);
    assert!((right(&s, 1) - 40.0).abs() < 1e-9);
    round_trip(&s);
}
#[test]
fn japanese_expands_clusters_and_distinct_distributed_includes_final_line() {
    let s = shape(&paragraph("just", "日本語本文資料"), 70);
    assert_eq!(s.lines.len(), 2);
    assert!((right(&s, 0) - 70.0).abs() < 1e-9);
    assert!(right(&s, 1) < 70.0);
    round_trip(&s);
    let s = shape(&paragraph("dist", "ABC"), 90);
    assert_eq!(s.lines[0].runs.len(), 3);
    assert!((s.lines[0].runs[1].x - 41.0).abs() < 1e-9);
    assert!((right(&s, 0) - 90.0).abs() < 1e-9);
    round_trip(&s);
    let just = shape(&paragraph("just", "ABC"), 90);
    assert_eq!(just.lines[0].runs.len(), 1);
    assert_eq!(right(&just, 0), 24.0);
}
#[test]
fn explicit_breaks_empty_and_unstretchable_lines_do_not_expand_for_justify() {
    let s = shape(&paragraph("just", "one two\n\nthree four five"), 100);
    assert_eq!(right(&s, 0), 56.0);
    assert!(s.lines[1].runs.is_empty());
    assert_eq!(right(&s, s.lines.len() - 1), 32.0);
    round_trip(&s);
    let s = shape(&paragraph("just", "abcdefghijklm"), 40);
    assert!(s.lines.iter().all(|l| l.runs.len() == 1));
    round_trip(&s);
}
#[test]
fn run_boundaries_do_not_make_latin_slots_and_metadata_survives_split() {
    let s = shape(
        r#"<a:p><a:pPr algn="dist"/><a:r><a:rPr sz="1200" b="1" u="sng"><a:solidFill><a:srgbClr val="FF0000"/></a:solidFill><a:hlinkClick r:id="rId1" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/></a:rPr><a:t>日</a:t></a:r><a:r><a:rPr sz="1200" i="1"/><a:t>本</a:t></a:r></a:p>"#,
        100,
    );
    let first = &s.lines[0].runs[0];
    assert!(first.bold && first.underline);
    assert_eq!(first.color, "#FF0000");
    assert!(s.lines[0].runs[1].italic);
    assert_eq!(first.source_offsets, vec![0, 1]);
    round_trip(&s);
    let s = shape(
        r#"<a:p><a:pPr algn="just"/><a:r><a:rPr sz="1200"/><a:t>one</a:t></a:r><a:r><a:rPr sz="1200" b="1"/><a:t>two three four</a:t></a:r></a:p>"#,
        110,
    );
    assert_eq!(s.lines[0].runs[0].width, 24.0);
    assert_eq!(s.lines[0].runs[1].x, 24.0);
    assert!((right(&s, 0) - 110.0).abs() < 1e-9);
    round_trip(&s);
}
#[test]
fn clusters_nbsp_and_cursive_letters_are_not_split_and_caps_stay_atomic() {
    let s = shape(&paragraph("dist", "a&#x301;👩‍💻b"), 180);
    let texts: Vec<_> = s.lines[0].runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(texts, vec!["a\u{301}", "👩‍💻", "b"]);
    round_trip(&s);
    let s = shape(&paragraph("dist", "ابجد"), 180);
    assert_eq!(s.lines[0].runs.len(), 1);
    round_trip(&s);
    let s = shape(&paragraph("just", "a&#xA0;b c d e"), 65);
    assert!(s.lines[0].runs.iter().any(|r| r.text.contains("a\u{a0}b")));
    round_trip(&s);
    let s = shape(
        r#"<a:p><a:pPr algn="dist"/><a:r><a:rPr cap="all" sz="1200"/><a:t>ßa</a:t></a:r></a:p>"#,
        100,
    );
    assert_eq!(s.lines[0].runs[0].text, "SS");
    assert_eq!(
        s.selection_geometry(0, 0, 0, false, &HeuristicMeasure)
            .unwrap()
            .caret[0]
            .x,
        0.0
    );
    round_trip(&s);
}
#[test]
fn inherited_right_margin_and_alignment_bound_the_expanded_line() {
    let s = shape(
        r#"<a:lstStyle><a:lvl1pPr algn="dist" marL="95250" marR="190500"/></a:lstStyle><a:p><a:r><a:rPr sz="1200"/><a:t>日本</a:t></a:r></a:p><a:p><a:pPr marR="95250"/><a:r><a:rPr sz="1200"/><a:t>資料</a:t></a:r></a:p>"#,
        100,
    );
    assert_eq!(s.lines[0].runs[0].x, 10.0);
    assert_eq!(right(&s, 0), 80.0);
    assert_eq!(right(&s, 1), 90.0);
    round_trip(&s);
}
#[test]
fn bullet_marker_is_fixed_while_text_expands_in_the_hanging_region() {
    let s = shape(
        r#"<a:p><a:pPr algn="dist" marL="190500" indent="-95250" marR="95250"><a:buChar char="•"/></a:pPr><a:r><a:rPr sz="1200"/><a:t>日本</a:t></a:r></a:p>"#,
        100,
    );
    assert!(s.lines[0].runs[0].source_offsets.is_empty());
    assert_eq!(s.lines[0].runs[0].x, 10.0);
    assert_eq!(s.lines[0].runs[0].width, 8.0);
    assert_eq!(s.lines[0].runs[1].x, 20.0);
    assert_eq!(right(&s, 0), 90.0);
    round_trip(&s);
}

#[test]
fn expanded_carets_round_trip_with_rotation_flips_vertical_frames_and_group_axes() {
    use jade_pptx_layout::{PlacedGroupTransform, Rect};
    for vertical in [0.0, 90.0, 270.0] {
        for flip in [false, true] {
            let mut s = shape(&paragraph("dist", "a日本b"), 150);
            s.rotation_rad = 0.37;
            s.flip_h = flip;
            s.flip_v = !flip;
            s.text_rotation_deg = vertical;
            s.local_rect = Some(s.rect);
            s.group_transforms = vec![PlacedGroupTransform {
                frame: Rect {
                    x: 500.0,
                    y: 400.0,
                    width: 220.0,
                    height: 300.0,
                },
                child_frame: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 150.0,
                    height: 420.0,
                },
                rotation_rad: -0.23,
                flip_h: true,
                flip_v: false,
            }];
            round_trip(&s);
        }
    }
}
