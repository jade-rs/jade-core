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

/// y of the first glyph drawn as `text`.
fn glyph_y(ops: &[Op], text: &str) -> f64 {
    ops.iter()
        .find_map(|op| match op {
            Op::FillText(value, _, y) if value == text => Some(*y),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {text}"))
}

/// Two MS Mincho paragraphs, the first underlined with `kind`: how far the
/// second one's baseline sits below the first's.
fn underlined_pitch(kind: &str, options: &[&str]) -> f64 {
    let mincho = |underline: Option<&str>| RunProperties {
        font_size: Some(10.5),
        fonts: Some(jade_docx_reader::document::RunFonts {
            ascii: Some("ＭＳ 明朝".into()),
            h_ansi: Some("ＭＳ 明朝".into()),
            east_asia: Some("ＭＳ 明朝".into()),
            ..Default::default()
        }),
        underline: underline.map(|_| true),
        underline_kind: underline.map(Into::into),
        ..RunProperties::default()
    };
    let paragraph = |text: &str, underline: Option<&str>| Paragraph {
        runs: vec![run(text, Some(mincho(underline)))],
        properties: None,
        node_id: None,
        comment_ranges: Vec::new(),
        comment_markers: Vec::new(),
    };
    let mut document = document(paragraph("横浜", Some(kind)), options);
    document.east_asian_line_breaking.use_east_asian_layout = true;
    document
        .body
        .as_mut()
        .unwrap()
        .elements
        .push(BodyElement::from_paragraph(paragraph("市税", None)));
    let ops = ops(&document);
    glyph_y(&ops, "市") - glyph_y(&ops, "横")
}

#[test]
fn space_for_ul_deepens_only_a_line_whose_underline_leaves_its_descent() {
    let plain = underlined_pitch("single", &[]);
    // A single rule fits in MS Mincho's descent: Word keeps `yokohama_page1`'s
    // underlined lines at the 13.5pt pitch of the others.
    assert!(
        (underlined_pitch("single", &["spaceForUL"]) - plain).abs() < 0.01,
        "single underline"
    );
    // A double one does not, and the line grows to hold it.
    let double = underlined_pitch("double", &[]);
    assert!((double - plain).abs() < 0.01, "no room without spaceForUL");
    let spaced = underlined_pitch("double", &["spaceForUL"]);
    let font_px = 10.5 * 96.0 / 72.0;
    let descent_px = 74.0 / 256.0 * font_px;
    let expected =
        jade_docx_layout::line_metrics::underline_depth_px(font_px, Some("double")) - descent_px;
    assert!(expected > 0.5, "{expected}");
    assert!(
        (spaced - plain - expected).abs() < 0.6,
        "plain {plain}, spaced {spaced}, expected +{expected}"
    );
}

/// Renders `body_xml`, with `doNotUseHTMLParagraphAutoSpacing` when `fixed`,
/// and returns the baseline of every glyph drawn as one of `marks`.
fn auto_spacing_baselines(body_xml: &str, fixed: bool, marks: &[&str]) -> Vec<f64> {
    let data = super::section_tests::build_docx_with_body(body_xml);
    let word_doc = futures::executor::block_on(crate::word_document::WordDocument::load(
        &data,
        &Options::default(),
    ))
    .expect("load");
    let mut document = word_doc
        .document
        .as_ref()
        .expect("document")
        .as_ref()
        .clone();
    if fixed {
        document
            .compat
            .options
            .push(("doNotUseHTMLParagraphAutoSpacing".into(), true));
    }
    let ops = ops(&document);
    marks.iter().map(|mark| glyph_y(&ops, mark)).collect()
}

const AUTO_SPACING: &str =
    r#"<w:spacing w:before="100" w:beforeAutospacing="1" w:after="100" w:afterAutospacing="1"/>"#;

#[test]
fn html_auto_spacing_leaves_no_space_between_list_items() {
    let item = |text: &str| {
        format!(
            r#"<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>{AUTO_SPACING}</w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
        )
    };
    let plain = |text: &str| {
        format!(r#"<w:p><w:pPr>{AUTO_SPACING}</w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#)
    };
    let body = [plain("A"), item("B"), item("C"), plain("D"), plain("E")].concat();
    let html = auto_spacing_baselines(&body, false, &["A", "B", "C", "D", "E"]);
    let fixed = auto_spacing_baselines(&body, true, &["A", "B", "C", "D", "E"]);
    let auto_px = 14.0 * 96.0 / 72.0;
    let line = html[4] - html[3] - auto_px;
    // Between two list items the auto spacing goes, as between HTML <li>s…
    assert!((html[2] - html[1] - line).abs() < 0.5, "{html:?}");
    // …but the list keeps it above and below, as other paragraphs do.
    assert!((html[1] - html[0] - line - auto_px).abs() < 0.5, "{html:?}");
    assert!((html[3] - html[2] - line - auto_px).abs() < 0.5, "{html:?}");
    // doNotUseHTMLParagraphAutoSpacing keeps a fixed 14pt everywhere.
    assert!(
        (fixed[2] - fixed[1] - line - auto_px).abs() < 0.5,
        "{fixed:?}"
    );
}

#[test]
fn html_auto_spacing_drops_at_the_top_and_bottom_of_a_table_cell() {
    let cell_paragraph = |text: &str| {
        format!(r#"<w:p><w:pPr>{AUTO_SPACING}</w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#)
    };
    let body = format!(
        r#"<w:tbl><w:tblPr><w:tblW w:w="5000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="5000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="5000" w:type="dxa"/></w:tcPr>{}{}</w:tc></w:tr></w:tbl><w:p><w:r><w:t>Z</w:t></w:r></w:p>"#,
        cell_paragraph("P"),
        cell_paragraph("Q")
    );
    let html = auto_spacing_baselines(&body, false, &["P", "Q", "Z"]);
    let fixed = auto_spacing_baselines(&body, true, &["P", "Q", "Z"]);
    let auto_px = 14.0 * 96.0 / 72.0;
    // The first paragraph rises to the cell top and what follows the table
    // comes up by both the first paragraph's space before and the last one's
    // space after; the 14pt between the two paragraphs stays.
    assert!(
        (fixed[0] - html[0] - auto_px).abs() < 0.5,
        "{html:?} vs {fixed:?}"
    );
    assert!(((fixed[1] - fixed[0]) - (html[1] - html[0])).abs() < 0.5);
    assert!(
        (fixed[2] - html[2] - 2.0 * auto_px).abs() < 0.5,
        "{html:?} vs {fixed:?}"
    );
}

/// Character borders are drawn in table cells too, in their line style.
#[test]
fn a_character_border_in_a_table_cell_is_drawn_in_its_line_style() {
    let fills = |style: &str| {
        let body = format!(
            r#"<w:tbl><w:tblPr><w:tblW w:w="5000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="5000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="5000" w:type="dxa"/></w:tcPr><w:p><w:r><w:rPr><w:bdr w:val="{style}" w:sz="12" w:space="0" w:color="00FF00"/></w:rPr><w:t>boxed</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#
        );
        let data = super::section_tests::build_docx_with_body(&body);
        let word_doc = futures::executor::block_on(crate::word_document::WordDocument::load(
            &data,
            &Options::default(),
        ))
        .expect("load");
        let ops = ops(word_doc.document.as_ref().expect("document"));
        let mut green = false;
        ops.iter()
            .filter(|op| {
                if let Op::SetFillStyle(color) = op {
                    green = color.eq_ignore_ascii_case("#00ff00");
                }
                green && matches!(op, Op::FillRect(..))
            })
            .count()
    };
    assert_eq!(fills("single"), 4);
    assert_eq!(fills("double"), 8);
}
