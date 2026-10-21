//! Invariants that must hold for any slide, not just the fixtures: wrapped
//! lines fit their box, group spaces compose, geometry stays finite, numbering
//! counts, and table cells tile their grid without overlapping.

use jade_pptx_layout::{
    layout_slide, HeuristicMeasure, PlacedGeometry, PlacedShape, RunStyle, TextMeasure,
    TEXT_INSET_X_EMU,
};
use jade_pptx_reader::{
    emu_to_px, Bullet, CellBorders, GroupShape, GroupTransform, Shape, ShapeItem, SlideContent,
    SlideSize, Table, TableCell, TableRow, TextBody, TextParagraph, TextRun, ThemeColors,
    Transform,
};
use proptest::prelude::*;

fn transform(x: i64, y: i64, cx: i64, cy: i64) -> Transform {
    Transform {
        offset_emu: (x, y),
        extent_emu: (cx, cy),
        rotation_60000ths: 0,
        flip_h: false,
        flip_v: false,
    }
}

fn shape(transform: Transform) -> Shape {
    Shape {
        name: "s".to_string(),
        placeholder: None,
        transform: Some(transform),
        preset_geometry: Some("rect".to_string()),
        fill: None,
        outline: None,
        style: None,
        text: None,
        id: None,
        adjustments: Vec::new(),
        custom_geometry: None,
        image_rel_id: None,
        image_part: None,
        source_rect: None,
        shadow: None,
        effects: Default::default(),
    }
}

fn theme() -> ThemeColors {
    ThemeColors::default()
}

fn slide_size() -> SlideSize {
    SlideSize {
        width_emu: 12_192_000,
        height_emu: 6_858_000,
    }
}

fn content(shapes: Vec<ShapeItem>) -> SlideContent {
    SlideContent {
        shapes,
        ..SlideContent::default()
    }
}

fn run_width(text: &str, size_pt: f32) -> f64 {
    HeuristicMeasure.measure(
        text,
        &RunStyle {
            size_pt,
            bold: false,
            italic: false,
            font: None,
            east_asian_font: None,
        },
    )
}

fn assert_finite(placed: &PlacedShape) {
    for value in [
        placed.rect.x,
        placed.rect.y,
        placed.rect.width,
        placed.rect.height,
        placed.rotation_rad,
    ] {
        assert!(value.is_finite(), "{placed:?}");
    }
    for line in &placed.lines {
        assert!(line.top.is_finite() && line.height.is_finite(), "{line:?}");
        for run in &line.runs {
            assert!(run.x.is_finite() && run.baseline_y.is_finite(), "{run:?}");
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn wrapped_lines_never_exceed_the_usable_width(
        text in "[a-z あいうえお]{0,80}",
        box_cx in 400_000i64..4_000_000,
        size_pt in 8u32..40,
    ) {
        let mut boxed = shape(transform(0, 0, box_cx, 914_400));
        boxed.text = Some(TextBody {
            paragraphs: vec![TextParagraph {
                runs: vec![TextRun {
                    text: text.clone(),
                    size_pt: Some(size_pt as f32),
                    ..TextRun::default()
                }],
                ..TextParagraph::default()
            }],
            ..TextBody::default()
        });
        let laid_out = layout_slide(slide_size(), &content(vec![ShapeItem::Shape(boxed)]), &theme(), &HeuristicMeasure);
        let usable = emu_to_px(box_cx) - 2.0 * emu_to_px(TEXT_INSET_X_EMU);
        for line in &laid_out.shapes[0].lines {
            let line_text: String = line.runs.iter().map(|run| run.text.as_str()).collect();
            let width = run_width(&line_text, size_pt as f32);
            let graphemes = line_text.chars().count();
            prop_assert!(
                width <= usable + 1e-6 || graphemes <= 1,
                "line {line_text:?} is {width} wide in {usable}"
            );
            // Nothing is lost or invented by the wrap.
        }
        let rejoined: String = laid_out.shapes[0]
            .lines
            .iter()
            .map(|line| line.runs.iter().map(|run| run.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join(" ");
        // Soft wraps swallow the space they broke at and CJK breaks without
        // one, so compare with every space removed.
        let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        prop_assert_eq!(squeeze(&rejoined), squeeze(&text));
    }

    #[test]
    fn nested_groups_compose_the_same_as_the_closed_form(
        off1 in (-2_000_000i64..2_000_000, -2_000_000i64..2_000_000),
        ext1 in (1i64..3_000_000, 1i64..3_000_000),
        ch_off1 in (-2_000_000i64..2_000_000, -2_000_000i64..2_000_000),
        ch_ext1 in (1i64..3_000_000, 1i64..3_000_000),
        off2 in (-2_000_000i64..2_000_000, -2_000_000i64..2_000_000),
        ext2 in (1i64..3_000_000, 1i64..3_000_000),
        ch_off2 in (-2_000_000i64..2_000_000, -2_000_000i64..2_000_000),
        ch_ext2 in (1i64..3_000_000, 1i64..3_000_000),
        leaf in (-2_000_000i64..2_000_000, -2_000_000i64..2_000_000, 1i64..3_000_000, 1i64..3_000_000),
    ) {
        let inner = GroupShape {
            name: "inner".into(),
            transform: Some(GroupTransform {
                outer: transform(off2.0, off2.1, ext2.0, ext2.1),
                child_offset_emu: ch_off2,
                child_extent_emu: ch_ext2,
            }),
            children: vec![ShapeItem::Shape(shape(transform(leaf.0, leaf.1, leaf.2, leaf.3)))],
            id: None,
            diagram_data_rel_id: None,
        };
        let outer = GroupShape {
            name: "outer".into(),
            transform: Some(GroupTransform {
                outer: transform(off1.0, off1.1, ext1.0, ext1.1),
                child_offset_emu: ch_off1,
                child_extent_emu: ch_ext1,
            }),
            children: vec![ShapeItem::Group(inner)],
            id: None,
            diagram_data_rel_id: None,
        };
        let laid_out = layout_slide(slide_size(), &content(vec![ShapeItem::Group(outer)]), &theme(), &HeuristicMeasure);
        let placed = &laid_out.shapes[0];
        assert_finite(placed);

        let s2 = (ext2.0 as f64 / ch_ext2.0 as f64, ext2.1 as f64 / ch_ext2.1 as f64);
        let s1 = (ext1.0 as f64 / ch_ext1.0 as f64, ext1.1 as f64 / ch_ext1.1 as f64);
        let in_outer = (
            off2.0 as f64 + (leaf.0 - ch_off2.0) as f64 * s2.0,
            off2.1 as f64 + (leaf.1 - ch_off2.1) as f64 * s2.1,
        );
        let absolute = (
            off1.0 as f64 + (in_outer.0 - ch_off1.0 as f64) * s1.0,
            off1.1 as f64 + (in_outer.1 - ch_off1.1 as f64) * s1.1,
        );
        let expected_w = leaf.2 as f64 * s2.0 * s1.0;
        let expected_h = leaf.3 as f64 * s2.1 * s1.1;
        let px = |emu: f64| emu / 914_400.0 * 96.0;
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-6 * (1.0 + a.abs().max(b.abs()));
        prop_assert!(close(placed.rect.x, px(absolute.0)), "{} vs {}", placed.rect.x, px(absolute.0));
        prop_assert!(close(placed.rect.y, px(absolute.1)), "{} vs {}", placed.rect.y, px(absolute.1));
        prop_assert!(close(placed.rect.width, px(expected_w)));
        prop_assert!(close(placed.rect.height, px(expected_h)));
    }

    #[test]
    fn degenerate_transforms_still_lay_out_finitely(
        off in (any::<i32>(), any::<i32>()),
        ext in (0i64..2, 0i64..2),
        ch_ext in (0i64..2, 0i64..2),
        rot in any::<i32>(),
        text_len in 0usize..20,
    ) {
        let mut leaf = shape(Transform {
            offset_emu: (off.0 as i64, off.1 as i64),
            extent_emu: ext,
            rotation_60000ths: rot,
            flip_h: true,
            flip_v: true,
        });
        leaf.text = Some(TextBody {
            paragraphs: vec![TextParagraph {
                bullet: Some(Bullet::Char("•".into())),
                runs: vec![TextRun { text: "x".repeat(text_len), ..TextRun::default() }],
                ..TextParagraph::default()
            }],
            ..TextBody::default()
        });
        let group = GroupShape {
            name: "g".into(),
            transform: Some(GroupTransform {
                outer: transform(0, 0, ext.0, ext.1),
                child_offset_emu: (0, 0),
                child_extent_emu: ch_ext,
            }),
            children: vec![ShapeItem::Shape(leaf)],
            id: None,
            diagram_data_rel_id: None,
        };
        let laid_out = layout_slide(slide_size(), &content(vec![ShapeItem::Group(group)]), &theme(), &HeuristicMeasure);
        for placed in &laid_out.shapes {
            assert_finite(placed);
        }
    }

    #[test]
    fn auto_numbers_count_up_and_restart_after_a_shallower_paragraph(
        first_run in 1usize..6,
        second_run in 1usize..6,
    ) {
        let numbered = |level: u8| TextParagraph {
            level,
            bullet: Some(Bullet::AutoNumber { scheme: "arabicPeriod".into() }),
            runs: vec![TextRun { text: "item".into(), ..TextRun::default() }],
            ..TextParagraph::default()
        };
        let mut paragraphs: Vec<TextParagraph> = (0..first_run).map(|_| numbered(1)).collect();
        paragraphs.push(numbered(0));
        paragraphs.extend((0..second_run).map(|_| numbered(1)));

        let mut boxed = shape(transform(0, 0, 9_144_000, 6_000_000));
        boxed.text = Some(TextBody { paragraphs, ..TextBody::default() });
        let laid_out = layout_slide(slide_size(), &content(vec![ShapeItem::Shape(boxed)]), &theme(), &HeuristicMeasure);
        let markers: Vec<String> = laid_out.shapes[0]
            .lines
            .iter()
            .map(|line| line.runs[0].text.clone())
            .collect();
        let mut expected: Vec<String> = (1..=first_run).map(|n| format!("{n}.")).collect();
        expected.push("1.".into());
        expected.extend((1..=second_run).map(|n| format!("{n}.")));
        prop_assert_eq!(markers, expected);
    }

    #[test]
    fn table_cells_tile_the_grid_without_overlapping(
        widths in prop::collection::vec(100_000i64..2_000_000, 1..5),
        heights in prop::collection::vec(100_000i64..1_000_000, 1..5),
    ) {
        let rows = heights
            .iter()
            .map(|h| TableRow {
                height_emu: *h,
                cells: widths
                    .iter()
                    .map(|_| TableCell {
                        text: None,
                        fill: None,
                        borders: CellBorders::default(),
                        grid_span: 1,
                        row_span: 1,
                        h_merge: false,
                        v_merge: false,
                        anchor: None,
                        insets: Default::default(),
                    })
                    .collect(),
            })
            .collect();
        let table = Table {
            name: "t".into(),
            transform: Some(transform(100_000, 200_000, widths.iter().sum(), heights.iter().sum())),
            column_widths_emu: widths.clone(),
            rows,
            id: None,
            style: Default::default(),
        };
        let laid_out = layout_slide(slide_size(), &content(vec![ShapeItem::Table(table)]), &theme(), &HeuristicMeasure);
        let cells: Vec<&PlacedShape> = laid_out
            .shapes
            .iter()
            .filter(|s| s.geometry == PlacedGeometry::Rect)
            .collect();
        prop_assert_eq!(cells.len(), widths.len() * heights.len());

        let area: f64 = cells.iter().map(|c| c.rect.width * c.rect.height).sum();
        let expected_area = emu_to_px(widths.iter().sum()) * emu_to_px(heights.iter().sum());
        prop_assert!((area - expected_area).abs() < 1e-6 * expected_area.max(1.0));

        for (i, a) in cells.iter().enumerate() {
            for b in cells.iter().skip(i + 1) {
                let overlap_x = (a.rect.x + a.rect.width).min(b.rect.x + b.rect.width) - a.rect.x.max(b.rect.x);
                let overlap_y = (a.rect.y + a.rect.height).min(b.rect.y + b.rect.height) - a.rect.y.max(b.rect.y);
                prop_assert!(
                    overlap_x <= 1e-6 || overlap_y <= 1e-6,
                    "{} overlaps {}",
                    a.name,
                    b.name
                );
            }
        }
    }
}
