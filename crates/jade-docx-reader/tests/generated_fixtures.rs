//! Smoke tests for the generated DOCX fixtures under `public/fixtures/`.
//!
//! These fixtures are produced by `scripts/generate-fixtures.mjs` and
//! `scripts/generate-word-compat-comprehensive.py`, and
//! `scripts/generate-word-compat-edge-cases.py` and exercise
//! the Word-compatibility features implemented in the Rust/WASM editor. The
//! tests guard that the generated packages stay parseable and that the targeted
//! OOXML features survive a round-trip through the reader.

use std::io::Read;
use std::path::PathBuf;

use jade_docx_reader::{
    DocumentParser, FieldComparisonOperator, RunFieldKind, TableLayoutMode, TableRowHeightRule,
    ThemeColorTransform, TocOptions,
};

fn public_fixture(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(name);
    assert!(
        path.exists(),
        "public fixture not found: {}",
        path.display()
    );
    path
}

/// Reads `word/document.xml` out of a `.docx` (ZIP) fixture.
fn read_document_xml(name: &str) -> String {
    read_part_xml(name, "word/document.xml")
}

/// Reads an arbitrary XML part out of a `.docx` (ZIP) fixture.
fn read_part_xml(name: &str, part: &str) -> String {
    let bytes = std::fs::read(public_fixture(name)).expect("read fixture bytes");
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open docx as zip archive");
    let mut file = archive
        .by_name(part)
        .unwrap_or_else(|_| panic!("{part} present"));
    let mut xml = String::new();
    file.read_to_string(&mut xml).expect("read document.xml");
    xml
}

#[test]
fn if_compare_fixture_preserves_fragmented_instructions_and_live_results() {
    let xml = read_document_xml("if-compare-field-edge-cases.docx");
    // Keep the field name split across `w:instrText` runs. This assertion
    // guards the generator itself so the fixture cannot silently become an
    // easier single-run parser case.
    assert!(xml.contains("> I<"));
    assert!(xml.contains(">F OrderTotal&gt;=100"));
    assert!(xml.contains("> COM<"));
    assert!(xml.contains(">PARE 2&lt;10 <"));
    // Preserve a true nested complex field: COMPARE is serialized between the
    // outer IF field's begin/separate boundaries, and its cached `1` is an
    // instruction operand rather than independently visible content.
    assert!(xml.contains("> IF <"));
    assert!(xml.contains("> COMPARE 2&lt;10 <"));
    assert!(xml.contains("> = 1 \"Nested true\" \"Nested false\" <"));
    assert!(
        xml.contains("<w:vanish/></w:rPr><w:t xml:space=\"preserve\">0</w:t>"),
        "nested COMPARE cache must stay deliberately stale"
    );
    assert!(xml.contains(">Nested bookmark operand: <"));
    assert!(xml.contains("> COMPARE OrderTotal&gt;=100 <"));
    assert!(xml.contains("> - Same-paragraph bookmark: <"));
    assert!(xml.contains("> COMPARE InlineTotal&gt;=100 <"));
    assert!(xml.contains(">Nested REF operand: <"));
    assert!(xml.contains("> REF OrderTotal <"));
    assert!(xml.contains(">Spaced nested REF right operand: <"));
    assert!(xml.contains("> REF RegionName <"));
    assert!(xml.contains(">Quote-bearing nested REF operand: <"));
    assert!(xml.contains("> REF QuotedRegion <"));
    assert!(xml.contains(
        "> = \"The \\\"quoted\\\" region\" \"Quoted value matched\" \"Quoted value rejected\" <"
    ));
    assert!(xml.contains(">Empty quoted operands: <"));
    assert!(xml.contains("> IF \"\" = \"\" \"Empty matched\" \"Empty rejected\" <"));
    assert!(xml.contains(">Compact empty operands: <"));
    assert!(xml.contains("> IF \"\"=\"\" \"Compact empty matched\" \"Compact empty rejected\" <"));
    assert!(xml.contains(">Nested empty result: <"));
    assert!(xml.contains("> IF 1=2 \"Unexpected inner\" \"\" <"));
    assert!(xml.contains(">Quoted numeric Expression2: <"));
    assert!(xml.contains("> IF 2 = \"02\" \"Numeric equal\" \"String differs\" <"));
    assert!(xml.contains(">Quoted bookmark-name literal: <"));
    assert!(xml
        .contains("> IF OrderTotal = \"OrderTotal\" \"Both dereferenced\" \"Literal differs\" <"));
    assert!(xml.contains(">Nested quoted numeric COMPARE: <"));
    assert!(xml.contains("> COMPARE 2 = \"02\" <"));
    assert!(xml.contains("> = 0 \"Quoted nested differs\" \"Stale quoted nested\" <"));
    assert!(xml.contains(">Quoted Expression1 literal: <"));
    assert!(xml.contains(
        "> IF \"OrderTotal\" = \"OrderTotal\" \"Quoted left literal\" \"Bookmark leaked\" <"
    ));
    assert!(xml.contains(">Nested quoted string COMPARE: <"));
    assert!(xml.contains("> COMPARE \"North Region\" = \"North Region\" <"));
    assert!(xml.contains("> = 1 \"Nested literal match\" \"Stale nested literal\" <"));
    assert!(xml.contains(">Forward bookmark nested COMPARE: <"));
    assert!(xml.contains("> COMPARE FutureTotal&gt;=100 <"));
    assert!(xml.contains("> = 1 \"Forward approved\" \"Stale forward cache\" <"));
    assert!(xml.contains(">Forward bookmark source follows its consuming field.<"));
    let over_limit_wildcard = "A".repeat(129);
    assert!(xml.contains(">Wildcard 128-character cap: <"));
    assert!(xml.contains(&format!(
        "> IF \"{over_limit_wildcard}\" = \"*\" \"Unlimited wildcard\" \"Wildcard capped\" <"
    )));

    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse IF/COMPARE fixture");
    let fields: Vec<_> = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .filter_map(jade_docx_reader::BodyElement::as_paragraph)
        .flat_map(|paragraph| &paragraph.runs)
        .filter_map(|run| run.field.as_ref())
        .collect();

    assert_eq!(fields.len(), 20, "parsed fields: {fields:#?}");
    assert!(matches!(
        fields[0].kind,
        RunFieldKind::If {
            operator: FieldComparisonOperator::GreaterOrEqual,
            ..
        }
    ));
    assert_eq!(fields[0].resolved_value.as_deref(), Some("Approved value"));
    // Word for Mac takes a nested COMPARE or IF at its saved result in the
    // main story and prints these instead; nested REFs stay live.
    for (index, expected) in [
        (4, "Nested false"),
        (5, "Bookmark denied"),
        (6, "Inline denied"),
        (12, "Nested empty rejected"),
        (17, "Stale nested literal"),
        (18, "Stale forward cache"),
    ] {
        assert_eq!(
            fields[index].word_mac_body_result.as_deref(),
            Some(expected),
            "field {index}"
        );
    }
    for index in [0, 7, 8, 9] {
        assert_eq!(fields[index].word_mac_body_result, None, "field {index}");
    }
    assert!(matches!(
        fields[1].kind,
        RunFieldKind::Compare {
            operator: FieldComparisonOperator::Equal,
            ..
        }
    ));
    assert_eq!(fields[1].resolved_value.as_deref(), Some("1"));
    assert_eq!(fields[2].resolved_value.as_deref(), Some("1"));
    assert_eq!(fields[3].resolved_value.as_deref(), Some(""));
    assert!(matches!(
        fields[4].kind,
        RunFieldKind::If {
            operator: FieldComparisonOperator::Equal,
            ..
        }
    ));
    assert_eq!(fields[4].resolved_value.as_deref(), Some("Nested true"));
    assert_eq!(
        fields[5].resolved_value.as_deref(),
        Some("Bookmark approved")
    );
    assert_eq!(fields[6].resolved_value.as_deref(), Some("Inline approved"));
    assert_eq!(fields[7].resolved_value.as_deref(), Some("REF approved"));
    assert_eq!(
        fields[8].kind,
        RunFieldKind::If {
            left: "North Region".to_string(),
            left_is_quoted: true,
            operator: FieldComparisonOperator::Equal,
            right: "North Region".to_string(),
            right_is_quoted: true,
            true_text: "Matched full region".to_string(),
            false_text: Some("Rejected full region".to_string()),
        }
    );
    assert_eq!(
        fields[8].resolved_value.as_deref(),
        Some("Matched full region")
    );
    assert_eq!(
        fields[9].kind,
        RunFieldKind::If {
            left: "The \"quoted\" region".to_string(),
            left_is_quoted: true,
            operator: FieldComparisonOperator::Equal,
            right: "The \"quoted\" region".to_string(),
            right_is_quoted: true,
            true_text: "Quoted value matched".to_string(),
            false_text: Some("Quoted value rejected".to_string()),
        }
    );
    assert_eq!(
        fields[9].resolved_value.as_deref(),
        Some("Quoted value matched")
    );
    assert_eq!(fields[10].resolved_value.as_deref(), Some("Empty matched"));
    assert_eq!(
        fields[11].resolved_value.as_deref(),
        Some("Compact empty matched")
    );
    assert_eq!(
        fields[12].resolved_value.as_deref(),
        Some("Nested empty matched")
    );
    assert_eq!(
        fields[13].kind,
        RunFieldKind::If {
            left: "2".to_string(),
            left_is_quoted: false,
            operator: FieldComparisonOperator::Equal,
            right: "02".to_string(),
            right_is_quoted: true,
            true_text: "Numeric equal".to_string(),
            false_text: Some("String differs".to_string()),
        }
    );
    assert_eq!(fields[13].resolved_value.as_deref(), Some("String differs"));
    assert_eq!(
        fields[14].kind,
        RunFieldKind::If {
            left: "OrderTotal".to_string(),
            left_is_quoted: false,
            operator: FieldComparisonOperator::Equal,
            right: "OrderTotal".to_string(),
            right_is_quoted: true,
            true_text: "Both dereferenced".to_string(),
            false_text: Some("Literal differs".to_string()),
        }
    );
    assert_eq!(
        fields[14].resolved_value.as_deref(),
        Some("Literal differs")
    );
    assert_eq!(
        fields[15].kind,
        RunFieldKind::If {
            left: "0".to_string(),
            left_is_quoted: false,
            operator: FieldComparisonOperator::Equal,
            right: "0".to_string(),
            right_is_quoted: false,
            true_text: "Quoted nested differs".to_string(),
            false_text: Some("Stale quoted nested".to_string()),
        }
    );
    assert_eq!(
        fields[15].resolved_value.as_deref(),
        Some("Quoted nested differs")
    );
    assert_eq!(
        fields[16].kind,
        RunFieldKind::If {
            left: "OrderTotal".to_string(),
            left_is_quoted: true,
            operator: FieldComparisonOperator::Equal,
            right: "OrderTotal".to_string(),
            right_is_quoted: true,
            true_text: "Quoted left literal".to_string(),
            false_text: Some("Bookmark leaked".to_string()),
        }
    );
    assert_eq!(
        fields[16].resolved_value.as_deref(),
        Some("Quoted left literal")
    );
    assert_eq!(
        fields[17].kind,
        RunFieldKind::If {
            left: "1".to_string(),
            left_is_quoted: false,
            operator: FieldComparisonOperator::Equal,
            right: "1".to_string(),
            right_is_quoted: false,
            true_text: "Nested literal match".to_string(),
            false_text: Some("Stale nested literal".to_string()),
        }
    );
    assert_eq!(
        fields[17].resolved_value.as_deref(),
        Some("Nested literal match")
    );
    assert_eq!(
        fields[18].kind,
        RunFieldKind::If {
            left: "1".to_string(),
            left_is_quoted: false,
            operator: FieldComparisonOperator::Equal,
            right: "1".to_string(),
            right_is_quoted: false,
            true_text: "Forward approved".to_string(),
            false_text: Some("Stale forward cache".to_string()),
        }
    );
    assert_eq!(
        fields[18].resolved_value.as_deref(),
        Some("Forward approved")
    );
    assert_eq!(
        fields[19].resolved_value.as_deref(),
        Some("Wildcard capped")
    );
}

#[test]
fn line_numbering_fixture_parses_section_line_numbering() {
    let xml = read_document_xml("line-numbering.docx");
    let parser = DocumentParser::new();
    let document = parser.parse_document_xml(&xml).expect("parse document");

    let section = document
        .default_section
        .as_ref()
        .expect("default section present");
    let line_numbering = section
        .line_numbering
        .as_ref()
        .expect("section carries w:lnNumType");

    assert_eq!(line_numbering.count_by, Some(1));
    assert_eq!(line_numbering.start, Some(1));
    assert_eq!(line_numbering.restart.as_deref(), Some("continuous"));
    assert_eq!(line_numbering.distance, Some(360));

    // The body should hold the five numbered paragraphs.
    let body = document.body.as_ref().expect("document body present");
    let paragraph_count = body
        .elements
        .iter()
        .filter(|el| matches!(el, jade_docx_reader::BodyElement::Paragraph(_)))
        .count();
    assert_eq!(paragraph_count, 5);
}

#[test]
fn char_effects_fixture_parses_run_decorations() {
    let xml = read_document_xml("char-effects.docx");
    let parser = DocumentParser::new();
    let document = parser.parse_document_xml(&xml).expect("parse document");

    let body = document.body.as_ref().expect("document body present");
    let paragraphs: Vec<&jade_docx_reader::Paragraph> = body
        .elements
        .iter()
        .filter_map(|el| match el {
            jade_docx_reader::BodyElement::Paragraph(p) => Some(p.as_ref()),
            _ => None,
        })
        .collect();

    // shadow / outline / emboss / imprint on the first four paragraphs.
    let first_run_props = |idx: usize| {
        paragraphs[idx].runs[0]
            .properties
            .as_ref()
            .expect("run has properties")
    };
    assert_eq!(first_run_props(0).shadow, Some(true), "shadow paragraph");
    assert_eq!(first_run_props(1).outline, Some(true), "outline paragraph");
    assert_eq!(first_run_props(2).emboss, Some(true), "emboss paragraph");
    assert_eq!(first_run_props(3).imprint, Some(true), "imprint paragraph");

    // The vanish paragraph: middle run is hidden.
    let vanish_para = paragraphs[4];
    let hidden = vanish_para
        .runs
        .iter()
        .find(|r| r.properties.as_ref().and_then(|p| p.vanish) == Some(true))
        .expect("a run is marked vanish");
    assert_eq!(hidden.text, "hidden");

    // The position paragraph: raised (+6pt) and lowered (-4pt) runs.
    let position_para = paragraphs[5];
    let positions: Vec<Option<f32>> = position_para
        .runs
        .iter()
        .map(|r| r.properties.as_ref().and_then(|p| p.text_position))
        .collect();
    assert!(
        positions.contains(&Some(6.0)),
        "raised run at +6pt present: {positions:?}"
    );
    assert!(
        positions.contains(&Some(-4.0)),
        "lowered run at -4pt present: {positions:?}"
    );
}

#[test]
fn comprehensive_word_compat_fixture_preserves_fields_tables_and_objects() {
    let xml = read_document_xml("word-compat-comprehensive.docx");
    let parser = DocumentParser::new();
    let document = parser.parse_document_xml(&xml).expect("parse document");

    let instructions = &document.toc_fields;
    assert_eq!(instructions.len(), 7, "all TOC samples are captured");
    assert!(instructions
        .iter()
        .any(|toc| toc.hide_page_number_levels == Some((2, 3))));
    assert!(instructions
        .iter()
        .any(|toc| toc.page_number_separator.as_deref() == Some("-")));
    assert!(instructions
        .iter()
        .any(|toc| toc.preserve_tabs && toc.preserve_line_breaks));
    assert!(instructions
        .iter()
        .any(|toc| toc.bookmark_name.as_deref() == Some("PartOne")));
    assert!(instructions.iter().any(|toc| {
        toc.caption_label.as_deref() == Some("Figure") && !toc.omit_caption_label_and_number
    }));
    assert!(instructions.iter().any(|toc| {
        toc.caption_label.as_deref() == Some("Figure") && toc.omit_caption_label_and_number
    }));

    let body = document.body.as_ref().expect("document body present");
    assert!(
        body.elements
            .iter()
            .any(|element| matches!(element, jade_docx_reader::BodyElement::Table(_))),
        "fixture contains a real table",
    );
    let paragraphs = body.elements.iter().filter_map(|element| match element {
        jade_docx_reader::BodyElement::Paragraph(paragraph) => Some(paragraph.as_ref()),
        _ => None,
    });
    let mut has_image = false;
    let mut has_text_box = false;
    for paragraph in paragraphs {
        has_image |= paragraph.runs.iter().any(|run| run.image.is_some());
        has_text_box |= paragraph.runs.iter().any(|run| run.text_box.is_some());
    }
    assert!(has_image, "fixture contains an inline image run");
    assert!(has_text_box, "fixture contains a VML textbox run");
}

#[test]
fn focused_toc_edge_fixtures_preserve_fragmented_instructions_and_caption_modes() {
    // Word is free to split one field instruction across several w:instrText
    // runs.  The fixture deliberately splits both keywords and quoted values,
    // guarding against parsers that inspect each run in isolation.
    let xml = read_document_xml("toc-instrtext-edge-cases.docx");
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse fragmented InstrText fixture");
    assert_eq!(document.toc_fields.len(), 2);
    let scoped = &document.toc_fields[0];
    assert_eq!(scoped.bookmark_name.as_deref(), Some("ScopeEdge"));
    assert_eq!(scoped.page_number_separator.as_deref(), Some("→"));
    assert_eq!(scoped.hide_page_number_levels, Some((2, 3)));
    assert_eq!((scoped.outline_min, scoped.outline_max), (1, 3));
    assert!(scoped.hyperlinks);
    assert!(scoped.preserve_tabs && scoped.preserve_line_breaks);
    let scope_bookmark = document
        .bookmarks
        .iter()
        .find(|bookmark| bookmark.name == "ScopeEdge")
        .expect("scope bookmark");
    assert_eq!(scope_bookmark.start_offset, 0);
    assert_eq!(scope_bookmark.end_offset, "Bookmark-scoped source".len());
    assert_eq!(
        document.toc_fields[1].custom_style_levels,
        vec![("Heading 1".to_string(), 1), ("Heading 2".to_string(), 2)]
    );
    assert!(document.toc_fields[1].hyperlinks);

    // Word's chapter-style page numbers combine two independent field types:
    // `TOC \\s` selects the most recent matching SEQ result and `\\d` chooses
    // the delimiter before PAGEREF. Splitting both the switch and identifier
    // ensures aggregation happens before switch parsing.
    let xml = read_document_xml("toc-sequence-page-number.docx");
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse sequence-prefixed TOC fixture");
    assert_eq!(document.toc_fields.len(), 1);
    let sequence = document.toc_fields[0]
        .sequence_page_number
        .as_ref()
        .expect("TOC sequence page-number options");
    assert_eq!(sequence.identifier, "Chapter");
    assert_eq!(sequence.separator.as_deref(), Some(":"));

    // Caption TOCs have two similar but semantically distinct switches: \c
    // keeps the label/number while \a emits description text only.
    let xml = read_document_xml("toc-caption-edge-cases.docx");
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse caption fixture");
    assert_eq!(document.toc_fields.len(), 2);
    assert!(document.toc_fields.iter().any(|toc| {
        toc.caption_label.as_deref() == Some("Figure") && !toc.omit_caption_label_and_number
    }));
    assert!(document.toc_fields.iter().any(|toc| {
        toc.caption_label.as_deref() == Some("Figure") && toc.omit_caption_label_and_number
    }));
}

#[test]
fn focused_toc_direct_outline_fixture_requires_u_for_direct_formatting() {
    let xml = read_document_xml("toc-direct-outline-levels.docx");
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse direct-outline TOC fixture");

    assert_eq!(document.toc_fields.len(), 2);
    let without_u = &document.toc_fields[0];
    let with_u = &document.toc_fields[1];
    assert!(!without_u.use_applied_outline_levels);
    assert!(with_u.use_applied_outline_levels);

    // The fixture deliberately keeps `w:outlineLvl` directly on a Normal
    // paragraph. The first TOC must see only the Heading 1 source, while the
    // second opts the direct level into the same source scan via `\\u`.
    let entries_without_u: Vec<(String, u8)> = document
        .generate_outline(without_u)
        .into_iter()
        .map(|entry| (entry.text, entry.level))
        .collect();
    assert_eq!(entries_without_u, vec![("Styled heading".to_string(), 1)]);

    let entries_with_u: Vec<(String, u8)> = document
        .generate_outline(with_u)
        .into_iter()
        .map(|entry| (entry.text, entry.level))
        .collect();
    assert_eq!(
        entries_with_u,
        vec![
            ("Direct outline paragraph".to_string(), 2),
            ("Styled heading".to_string(), 1)
        ]
    );
}

#[test]
fn focused_tc_field_fixture_filters_hidden_entries_by_type_and_level() {
    let xml = read_document_xml("toc-tc-field-edge-cases.docx");
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse TC field fixture");

    assert_eq!(document.toc_fields.len(), 1);
    assert_eq!(document.toc_entry_fields.len(), 5);
    let options = &document.toc_fields[0];
    assert!(options.use_toc_entry_fields);
    assert_eq!(options.toc_entry_identifier.as_deref(), Some("i"));
    assert_eq!(options.toc_entry_levels, Some((2, 3)));
    let outline = document.generate_outline(options);
    let entries: Vec<(&str, u8)> = outline
        .iter()
        .map(|entry| (entry.text.as_str(), entry.level))
        .collect();
    assert_eq!(
        entries,
        vec![("Architecture overview", 2), ("Deployment map", 3)]
    );
    assert!(!outline[0].hide_page_number);
    assert!(outline[1].hide_page_number);

    // An untyped TC field participates in the ordinary heading TOC even
    // without `TOC \\f`. Typed illustration/table fields remain isolated.
    let ordinary = document.generate_outline(&TocOptions::default());
    let ordinary_entries: Vec<(&str, u8, bool)> = ordinary
        .iter()
        .map(|entry| (entry.text.as_str(), entry.level, entry.hide_page_number))
        .collect();
    assert_eq!(
        ordinary_entries,
        vec![
            ("Generated contents (TOC \\f i \\l 2-3)", 1, false),
            ("Manual default entry", 2, true),
        ]
    );
}

#[test]
fn focused_part_and_object_fixtures_retain_boundary_shapes() {
    // Header/footer parts are outside document.xml.  Assert their raw OOXML so
    // future fixture regeneration cannot silently flatten split runs or fields.
    let header = read_part_xml("header-footer-edge-cases.docx", "word/header1.xml");
    assert!(header.contains(">Inter<"));
    assert!(header.contains(">op<"));
    assert!(header.contains(">erability<"));
    assert!(header.contains("<w:tab"));
    assert!(header.contains("<w:tbl"));
    assert!(header.contains(">Table left<"));
    assert!(header.contains(">Table center<"));
    let footer = read_part_xml("header-footer-edge-cases.docx", "word/footer1.xml");
    assert!(footer.contains("PAGE \\* MERGEFORMAT"));
    assert!(footer.contains("> NUM<"));
    assert!(footer.contains(">PAGES <"));
    assert!(footer.contains("w:fldLock=\"true\""));

    let xml = read_document_xml("table-object-edge-cases.docx");
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse table/object fixture");
    let body = document.body.as_ref().expect("document body present");
    assert!(body
        .elements
        .iter()
        .any(|element| matches!(element, jade_docx_reader::BodyElement::Table(_))));
    let paragraphs = body.elements.iter().filter_map(|element| match element {
        jade_docx_reader::BodyElement::Paragraph(paragraph) => Some(paragraph.as_ref()),
        _ => None,
    });
    let (mut has_textbox, mut has_image) = (false, false);
    for paragraph in paragraphs {
        has_textbox |= paragraph.runs.iter().any(|run| run.text_box.is_some());
        has_image |= paragraph.runs.iter().any(|run| run.image.is_some());
    }
    assert!(has_textbox, "fixture contains a VML textbox");
    // The image is nested inside a table cell, so verify the DrawingML package
    // shape in addition to reader traversal of top-level body paragraphs.
    assert!(has_image || xml.contains("<w:drawing>"));
    assert!(xml.contains("<w:gridSpan"), "fixture contains merged cells");
    assert!(
        xml.matches("<w:tbl>").count() >= 2,
        "fixture contains a nested table"
    );
}

#[test]
fn document_variable_fixture_retains_settings_values_and_fragmented_fields() {
    let settings = read_part_xml("document-variable-fields.docx", "word/settings.xml");
    assert!(settings.contains("<w:docVars>"));
    assert!(settings.contains("w:name=\"Release Channel\""));
    assert!(settings.contains("w:val=\"Beta &amp; Preview\""));

    // Check all three Word stories because header/footer fields are stored in
    // separate package parts and can otherwise regress independently of body
    // instruction parsing.
    let document = read_document_xml("document-variable-fields.docx");
    let header = read_part_xml("document-variable-fields.docx", "word/header1.xml");
    let footer = read_part_xml("document-variable-fields.docx", "word/footer1.xml");
    assert!(document.contains("> docvar<"));
    assert!(document.contains(">iable \"Release Channel\" <"));
    assert!(header.contains("> DOCVAR<"));
    assert!(header.contains(">IABLE \"Release Channel\" <"));
    assert!(footer.contains("Stale Footer Build"));
    assert!(document.contains("Stale Table Channel"));
    assert!(document.contains("DocVariableTextBox"));
    assert!(document.contains("Stale Textbox Channel"));
}

#[test]
fn document_property_fixture_retains_extended_properties_and_header_field() {
    let properties = read_part_xml("document-property-fields.docx", "docProps/app.xml");
    assert!(properties.contains("<Company>Analytical Engines &amp; Co.</Company>"));
    assert!(properties.contains("<Manager>Grace Hopper</Manager>"));

    let header = read_part_xml("document-property-fields.docx", "word/header1.xml");
    assert!(header.contains("DOCPROPERTY"));
    assert!(header.contains("\"Company\""));
    assert!(header.contains("Stale Header Company"));
}

#[test]
fn bidi_visual_fixture_preserves_logical_cells_and_mirrored_table_semantics() {
    let xml = read_document_xml("table-bidi-visual.docx");
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse bidiVisual table fixture");
    let table = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .find_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("fixture table");

    assert!(
        table.bidi_visual,
        "w:bidiVisual enables mirrored grid rendering"
    );
    assert_eq!(table.columns.len(), 3);
    assert_eq!(table.columns[0].width, Some(72.0));
    assert_eq!(table.columns[1].width, Some(126.0));
    assert_eq!(table.columns[2].width, Some(180.0));
    assert_eq!(table.rows[2].cells[0].span, Some(2));
    let margins = table
        .cell_margins
        .as_ref()
        .expect("logical table cell margins");
    assert_eq!(margins.start, Some(18.0));
    assert_eq!(margins.end, Some(3.6));
    assert_eq!(margins.physical_left(true), Some(3.6));
    assert_eq!(margins.physical_right(true), Some(18.0));
    let borders = table.borders.as_ref().expect("logical table borders");
    assert_eq!(
        borders
            .physical_left(true)
            .and_then(|edge| edge.color.as_deref()),
        Some("#7030A0")
    );
    assert_eq!(
        borders
            .physical_right(true)
            .and_then(|edge| edge.color.as_deref()),
        Some("#00B050")
    );
    assert_eq!(
        table.rows[1].cells[0]
            .style
            .get("border-start-style")
            .map(String::as_str),
        Some("double")
    );
    assert_eq!(
        table.rows[1].cells[2]
            .style
            .get("border-end-color")
            .map(String::as_str),
        Some("#ED7D31")
    );
}

#[test]
fn bidi_visual_alignment_fixture_preserves_justification_and_leading_indent() {
    let xml = read_document_xml("table-bidi-alignment.docx");
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse bidiVisual alignment fixture");
    let tables: Vec<_> = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .filter_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .collect();

    assert_eq!(tables.len(), 5);
    assert!(tables.iter().all(|table| table.bidi_visual));
    assert_eq!(
        tables[0].style.get("text-align").map(String::as_str),
        Some("left")
    );
    assert_eq!(
        tables[0].style.get("margin-left").map(String::as_str),
        Some("36.00pt")
    );
    assert_eq!(
        tables[1].style.get("text-align").map(String::as_str),
        Some("center")
    );
    assert_eq!(
        tables[2].style.get("text-align").map(String::as_str),
        Some("right")
    );
    assert_eq!(
        tables[3].style.get("text-align").map(String::as_str),
        Some("left")
    );
    assert!(
        !tables[3].style.contains_key("margin-left"),
        "automatic tblInd is ignored rather than treated as centering"
    );
    assert!(
        !tables[4].style.contains_key("margin-left"),
        "percentage tblInd is ignored rather than becoming a physical offset"
    );
}

#[test]
fn table_cell_spacing_fixture_preserves_table_default_and_row_overrides() {
    let xml = read_document_xml("table-cell-spacing.docx");
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse table cell spacing fixture");
    let table = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .find_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("fixture table");

    // 144/60/120 dxa convert to 7.2/3/6pt. Row three obtains its value from
    // tblPrEx, the container Word uses for style-derived row exceptions. The
    // explicit nil on row four is intentionally Some(0): treating it as None
    // would incorrectly inherit the table default and leave a visible gap.
    assert_eq!(table.cell_spacing, Some(7.2));
    assert_eq!(table.rows[0].cell_spacing, None);
    assert_eq!(table.rows[1].cell_spacing, Some(3.0));
    assert_eq!(table.rows[2].cell_spacing, None);
    assert_eq!(table.rows[2].cell_spacing_exception, Some(6.0));
    let exception_margins = table.rows[2]
        .cell_margins
        .as_ref()
        .expect("tblPrEx cell margins");
    assert_eq!(exception_margins.start, Some(18.0));
    assert_eq!(exception_margins.end, Some(2.0));
    assert_eq!(exception_margins.top, Some(6.0));
    assert_eq!(exception_margins.bottom, Some(2.0));
    assert_eq!(
        table.cell_style.get("background-color").map(String::as_str),
        Some("#DDEBF7")
    );
    assert_eq!(
        table.rows[2]
            .style
            .get("background-color")
            .map(String::as_str),
        Some("#FFF2CC")
    );
    assert_eq!(
        table.rows[2].cells[1]
            .style
            .get("background-color")
            .map(String::as_str),
        Some("#FCE4D6")
    );
    let table_inside_v = table
        .borders
        .as_ref()
        .and_then(|borders| borders.inside_v.as_ref())
        .expect("table inside vertical border");
    assert_eq!(table_inside_v.color.as_deref(), Some("#4472C4"));
    let exception_inside_v = table.rows[2]
        .borders
        .as_ref()
        .and_then(|borders| borders.inside_v.as_ref())
        .expect("tblPrEx inside vertical border");
    assert_eq!(exception_inside_v.val.as_deref(), Some("double"));
    assert_eq!(exception_inside_v.size_eighths, Some(16));
    assert_eq!(exception_inside_v.color.as_deref(), Some("#C00000"));
    assert_eq!(table.rows[3].cell_spacing, Some(0.0));
}

#[test]
fn table_cell_preferred_width_fixture_omits_grid_but_retains_tc_widths() {
    let xml = read_document_xml("table-cell-preferred-widths.docx");
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse cell preferred width fixture");
    let table = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .find_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("fixture table");

    assert!(
        table.columns.is_empty(),
        "fixture must exercise tcW without gridCol fallback"
    );
    let first_row_widths: Vec<_> = table.rows[0]
        .cells
        .iter()
        .map(|cell| cell.style.get("width").map(String::as_str))
        .collect();
    assert_eq!(
        first_row_widths,
        vec![Some("72.00pt"), Some("144.00pt"), Some("72.00pt")]
    );
    assert_eq!(table.rows[1].cells[0].span, Some(2));
    assert_eq!(
        table.rows[1].cells[0]
            .style
            .get("width")
            .map(String::as_str),
        Some("216.00pt")
    );
    assert_eq!(table.rows[2].grid_before, Some(1));
    assert_eq!(table.rows[2].width_before.as_deref(), Some("72.00pt"));
    assert_eq!(table.rows[2].cells.len(), 1);
    assert_eq!(
        table.rows[2].cells[0]
            .style
            .get("width")
            .map(String::as_str),
        Some("144.00pt")
    );
    assert_eq!(table.rows[2].grid_after, Some(1));
    assert_eq!(table.rows[2].width_after.as_deref(), Some("72.00pt"));

    let tables: Vec<_> = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .filter_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .collect();
    let budget_table = tables.get(1).expect("partial-grid tblW fixture table");
    assert_eq!(
        budget_table.style.get("width").map(String::as_str),
        Some("50.00%")
    );
    assert_eq!(budget_table.columns.len(), 1);
    assert_eq!(budget_table.columns[0].width, Some(72.0));
    assert!(budget_table.rows[0]
        .cells
        .iter()
        .all(|cell| !cell.style.contains_key("width")));
}

#[test]
fn table_diagonal_border_fixture_preserves_both_physical_directions() {
    let xml = read_document_xml("table-diagonal-borders.docx");
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse diagonal border fixture");
    let table = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .find_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("fixture table");

    let descending = &table.rows[0].cells[0].style;
    assert_eq!(
        descending.get("border-tl2br-style").map(String::as_str),
        Some("double")
    );
    assert_eq!(
        descending.get("border-tl2br-color").map(String::as_str),
        Some("#C00000")
    );
    let ascending = &table.rows[0].cells[1].style;
    assert_eq!(
        ascending.get("border-tr2bl-style").map(String::as_str),
        Some("dashed")
    );
    let crossed = &table.rows[1].cells[0].style;
    assert!(crossed.contains_key("border-tl2br-style"));
    assert!(crossed.contains_key("border-tr2bl-style"));
    let control = &table.rows[1].cells[1].style;
    assert!(!control.contains_key("border-tl2br-style"));
    assert!(!control.contains_key("border-tr2bl-style"));
}

#[test]
fn table_border_conflict_fixture_preserves_visible_and_hidden_opponents() {
    let xml = read_document_xml("table-border-conflicts.docx");
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse shared border conflict fixture");
    let tables: Vec<_> = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .filter_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .collect();

    let ltr = tables.first().expect("LTR conflict table");
    assert_eq!(
        ltr.rows[0].cells[0]
            .style
            .get("border-right-style")
            .map(String::as_str),
        Some("double")
    );
    assert_eq!(
        ltr.rows[0].cells[1]
            .style
            .get("border-left-style")
            .map(String::as_str),
        Some("none")
    );
    assert_eq!(
        ltr.rows[1].cells[1]
            .style
            .get("border-left-style")
            .map(String::as_str),
        Some("dashed")
    );
    assert_eq!(
        ltr.rows[2].cells[0]
            .style
            .get("border-right-width")
            .map(String::as_str),
        Some("4.0000pt")
    );
    assert_eq!(
        ltr.rows[2].cells[1]
            .style
            .get("border-left-style")
            .map(String::as_str),
        Some("double")
    );
    assert_eq!(
        ltr.rows[3].cells[1]
            .style
            .get("border-left-color")
            .map(String::as_str),
        Some("#7030A0")
    );
    assert_eq!(
        ltr.rows[4].cells[0]
            .style
            .get("border-right-color")
            .map(String::as_str),
        Some("#FF0000")
    );
    assert_eq!(
        ltr.rows[4].cells[1]
            .style
            .get("border-left-color")
            .map(String::as_str),
        Some("#0000FF")
    );

    let horizontal = tables.get(1).expect("horizontal row conflict table");
    assert_eq!(horizontal.rows.len(), 2);
    assert_eq!(
        horizontal.rows[0].cells[0]
            .style
            .get("border-bottom-color")
            .map(String::as_str),
        Some("#00B050")
    );
    assert_eq!(
        horizontal.rows[1].cells[0]
            .style
            .get("border-top-style")
            .map(String::as_str),
        Some("double")
    );
    assert_eq!(
        horizontal.rows[1].cells[1]
            .style
            .get("border-top-color")
            .map(String::as_str),
        Some("#7030A0")
    );

    let segmented = tables.get(2).expect("segmented horizontal conflict table");
    assert_eq!(segmented.rows[0].cells[0].span, Some(2));
    assert_eq!(
        segmented.rows[0].cells[0]
            .style
            .get("border-bottom-color")
            .map(String::as_str),
        Some("#4472C4")
    );
    assert_eq!(
        segmented.rows[1].cells[0]
            .style
            .get("border-top-color")
            .map(String::as_str),
        Some("#00B050")
    );
    assert_eq!(
        segmented.rows[1].cells[1]
            .style
            .get("border-top-color")
            .map(String::as_str),
        Some("#7030A0")
    );

    let rtl = tables.get(3).expect("bidiVisual conflict table");
    assert!(rtl.bidi_visual);
    assert_eq!(
        rtl.rows[0].cells[0]
            .style
            .get("border-end-color")
            .map(String::as_str),
        Some("#00B050")
    );
    assert_eq!(
        rtl.rows[0].cells[1]
            .style
            .get("border-start-style")
            .map(String::as_str),
        Some("none")
    );
}

#[test]
fn table_hide_mark_fixture_preserves_cell_height_semantics() {
    let xml = read_document_xml("table-hide-mark.docx");
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse hideMark fixture");
    let table = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .find_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("fixture table");

    assert!(table.rows[0]
        .cells
        .iter()
        .all(|cell| !cell.style.contains_key("hide-mark")));
    assert!(table.rows[1].cells.iter().all(|cell| cell
        .style
        .get("hide-mark")
        .is_some_and(|value| value == "true")));
}

#[test]
fn table_legacy_hmerge_fixture_matches_modern_grid_span() {
    let xml = read_document_xml("table-legacy-hmerge.docx");
    assert!(xml.contains("<w:hMerge w:val=\"restart\"/>"));
    assert!(xml.contains("<w:hMerge/>"));

    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse legacy hMerge fixture");
    let table = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .find_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("fixture table");

    assert_eq!(table.rows[0].cells.len(), 2);
    assert_eq!(table.rows[0].cells[1].span, Some(2));
    assert_eq!(table.rows[1].cells.len(), 2);
    assert_eq!(table.rows[1].cells[1].span, Some(2));
    assert!(table.rows[1]
        .cells
        .iter()
        .all(|cell| !cell.style.contains_key("horizontal-merge")));
}

#[test]
fn table_no_wrap_autofit_fixture_preserves_absolute_and_percentage_constraints() {
    let xml = read_document_xml("table-nowrap-autofit.docx");
    assert!(xml.contains("<w:noWrap/>"));

    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse noWrap autofit fixture");
    let tables: Vec<_> = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .filter_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .collect();

    assert_eq!(tables.len(), 4);
    assert_eq!(tables[0].table_layout, Some(TableLayoutMode::Autofit));
    assert!(!tables[0].rows[0].cells[0].style.contains_key("no-wrap"));
    assert_eq!(
        tables[1].rows[0].cells[0]
            .style
            .get("no-wrap")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        tables[1].rows[0].cells[0]
            .style
            .get("width")
            .map(String::as_str),
        Some("300.00pt")
    );
    assert_eq!(tables[2].table_layout, Some(TableLayoutMode::Autofit));
    assert_eq!(
        tables[2].rows[0].cells[0]
            .style
            .get("no-wrap")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        tables[2].rows[0].cells[0]
            .style
            .get("width")
            .map(String::as_str),
        Some("50.00%")
    );
    assert_eq!(tables[3].table_layout, Some(TableLayoutMode::Autofit));
    assert!(tables[3].rows[0]
        .cells
        .iter()
        .all(|cell| !cell.style.contains_key("no-wrap")));
    let long_cell_text = tables[3].rows[0].cells[0].blocks[0]
        .as_paragraph()
        .expect("ordinary autofit cell paragraph")
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect::<String>();
    assert_eq!(long_cell_text, "XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX");
}

#[test]
fn table_floating_horizontal_fixture_preserves_position_over_inline_alignment() {
    let xml = read_document_xml("table-floating-horizontal.docx");
    assert!(xml.contains("<w:tblpPr"));
    assert!(xml.contains("w:tblpXSpec=\"right\""));
    assert!(xml.contains("<w:jc w:val=\"left\"/>"));

    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse floating table fixture");
    let table = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .find_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("floating fixture table");

    assert_eq!(
        table.style.get("table-position").map(String::as_str),
        Some("floating")
    );
    assert_eq!(
        table
            .style
            .get("table-position-horizontal-anchor")
            .map(String::as_str),
        Some("margin")
    );
    assert_eq!(
        table
            .style
            .get("table-position-x-align")
            .map(String::as_str),
        Some("right")
    );
    assert_eq!(
        table
            .style
            .get("table-position-vertical-anchor")
            .map(String::as_str),
        Some("page")
    );
    assert_eq!(
        table.style.get("table-position-y").map(String::as_str),
        Some("180.00pt")
    );
    assert_eq!(
        table.style.get("text-align").map(String::as_str),
        Some("left"),
        "ordinary jc remains available but tblpPr wins during rendering"
    );
}

#[test]
fn table_floating_overlap_fixture_preserves_pairwise_never_policy() {
    let xml = read_document_xml("table-floating-overlap.docx");
    assert_eq!(xml.matches("<w:tblpPr").count(), 2);
    assert!(xml.contains("<w:tblOverlap w:val=\"never\"/>"));

    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse floating overlap fixture");
    let tables = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .filter_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(tables.len(), 2);
    assert_eq!(tables[0].style.get("table-overlap"), None);
    assert_eq!(
        tables[1].style.get("table-overlap").map(String::as_str),
        Some("never")
    );
    assert_eq!(
        tables[0].style.get("table-position-x").map(String::as_str),
        tables[1].style.get("table-position-x").map(String::as_str)
    );
    assert_eq!(
        tables[0].style.get("table-position-y").map(String::as_str),
        tables[1].style.get("table-position-y").map(String::as_str)
    );
}

#[test]
fn table_floating_anchor_defaults_fixture_omits_both_anchor_attributes() {
    let xml = read_document_xml("table-floating-anchor-defaults.docx");
    assert!(xml.contains("<w:tblpPr w:tblpX=\"1800\" w:tblpY=\"3600\"/>"));
    assert!(!xml.contains("w:horzAnchor="));
    assert!(!xml.contains("w:vertAnchor="));

    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse floating anchor-default fixture");
    let table = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .find_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("floating anchor-default table");

    assert_eq!(
        table.style.get("table-position").map(String::as_str),
        Some("floating")
    );
    assert_eq!(
        table.style.get("table-position-x").map(String::as_str),
        Some("90.00pt")
    );
    assert_eq!(
        table.style.get("table-position-y").map(String::as_str),
        Some("180.00pt")
    );
    assert_eq!(table.style.get("table-position-horizontal-anchor"), None);
    assert_eq!(table.style.get("table-position-vertical-anchor"), None);
}

#[test]
fn table_floating_inline_vertical_fixture_preserves_relative_alignment() {
    let xml = read_document_xml("table-floating-inline-vertical.docx");
    assert!(xml.contains("w:vertAnchor=\"page\""));
    assert!(xml.contains("w:tblpYSpec=\"inline\""));
    assert!(xml.contains("w:tblpY=\"3600\""));

    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse floating inline-vertical fixture");
    let table = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .find_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("floating inline-vertical table");

    assert_eq!(
        table
            .style
            .get("table-position-y-align")
            .map(String::as_str),
        Some("inline")
    );
    assert_eq!(
        table.style.get("table-position-y").map(String::as_str),
        Some("180.00pt"),
        "the renderer, not the parser, resolves tblpYSpec precedence"
    );
}

#[test]
fn table_row_height_defaults_fixture_uses_word_omission_semantics() {
    let xml = read_document_xml("table-row-height-defaults.docx");
    assert!(xml.contains("<w:trHeight w:val=\"480\"/>"));
    assert!(xml.contains("<w:trHeight w:val=\"480\" w:hRule=\"exact\"/>"));
    assert!(xml.contains("<w:bottom w:w=\"240\" w:type=\"dxa\"/>"));

    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse row-height defaults fixture");
    let table = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .find_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("row-height defaults table");

    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.rows[0].height, Some(24.0));
    assert_eq!(table.rows[0].height_rule, Some(TableRowHeightRule::AtLeast));
    assert_eq!(table.rows[1].height, Some(24.0));
    assert_eq!(table.rows[1].height_rule, Some(TableRowHeightRule::Exact));
    assert_eq!(
        table.rows[1].cells[1]
            .margins
            .as_ref()
            .and_then(|margins| margins.bottom),
        Some(12.0)
    );
}

#[test]
fn table_cell_fit_text_fixture_preserves_enabled_and_disabled_cells() {
    let xml = read_document_xml("table-cell-fit-text.docx");
    assert_eq!(
        xml.matches("<w:tcFitText").count(),
        4,
        "fixture must cover enabled and explicit-false compression/expansion forms"
    );
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse table cell fit-text fixture");
    let table = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .find_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("fixture table");

    assert_eq!(
        table.rows[0].cells[0]
            .style
            .get("fit-text")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        table.rows[0].cells[1]
            .style
            .get("fit-text")
            .map(String::as_str),
        Some("false")
    );
    assert_eq!(table.rows[0].cells[0].blocks.len(), 1);
    assert_eq!(
        table.rows[1].cells[0]
            .style
            .get("fit-text")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        table.rows[1].cells[1]
            .style
            .get("fit-text")
            .map(String::as_str),
        Some("false")
    );
}

#[test]
fn table_repeating_header_fixture_parses_empty_on_and_explicit_false() {
    let xml = read_document_xml("table-repeating-header.docx");
    assert!(xml.contains("<w:tblHeader/>"));
    assert!(xml.contains("<w:tblHeader w:val=\"false\"/>"));
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse repeating table-header fixture");
    let table = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .find_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("fixture table");

    assert_eq!(table.rows.len(), 61);
    assert!(table.rows[0].is_header);
    assert!(!table.rows[1].is_header);
    assert!(table.rows[2..].iter().all(|row| !row.is_header));
}

#[test]
fn table_hidden_rows_fixture_preserves_top_level_and_nested_markers() {
    let xml = read_document_xml("table-hidden-rows.docx");
    assert_eq!(xml.matches("<w:hidden").count(), 3);
    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse hidden table-row fixture");
    let tables: Vec<_> = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .filter_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .collect();

    let top = tables[0];
    assert_eq!(top.rows.len(), 4);
    assert_eq!(
        top.rows[1].style.get("hidden").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        top.rows[2].style.get("hidden").map(String::as_str),
        Some("false")
    );

    let nested = tables[1].rows[0].cells[0]
        .blocks
        .iter()
        .find_map(|block| match block {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("nested fixture table");
    assert_eq!(
        nested.rows[1].style.get("hidden").map(String::as_str),
        Some("true")
    );
}

#[test]
fn textbox_body_properties_fixture_preserves_insets_and_vertical_anchor() {
    let xml = read_document_xml("textbox-body-properties.docx");
    assert!(xml.contains("lIns=\"190500\""));
    assert!(xml.contains("tIns=\"95250\""));
    assert!(xml.contains("rIns=\"285750\""));
    assert!(xml.contains("bIns=\"381000\""));
    assert!(xml.contains("anchor=\"ctr\""));

    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse textbox body-property fixture");
    let text_box = document
        .body
        .as_ref()
        .expect("document body")
        .elements
        .iter()
        .filter_map(|element| element.as_paragraph())
        .flat_map(|paragraph| paragraph.runs.iter())
        .find_map(|run| run.text_box.as_ref())
        .expect("DrawingML textbox");

    assert_eq!(text_box.text, "Centered by Word body properties");
    assert_eq!(text_box.insets.left_px, 20.0);
    assert_eq!(text_box.insets.top_px, 10.0);
    assert_eq!(text_box.insets.right_px, 30.0);
    assert_eq!(text_box.insets.bottom_px, 40.0);
    assert_eq!(
        text_box.vertical_alignment,
        jade_docx_reader::TextBoxVerticalAlignment::Center
    );
}

#[test]
fn shape_preset_fixture_keeps_common_textless_and_text_shapes() {
    let xml = read_document_xml("shape-preset-geometries.docx");
    assert!(xml.contains("prst=\"ellipse\""));
    assert!(xml.contains("prst=\"roundRect\""));
    assert!(xml.contains("prst=\"diamond\""));
    assert!(xml.contains("prst=\"triangle\""));
    assert!(xml.contains("prst=\"rtTriangle\""));
    assert!(xml.contains("prst=\"parallelogram\""));
    assert!(xml.contains("prst=\"trapezoid\""));
    assert!(xml.contains("prst=\"pentagon\""));
    assert!(xml.contains("prst=\"hexagon\""));
    assert!(xml.contains("prst=\"octagon\""));
    assert!(xml.contains("prst=\"plus\""));
    assert!(xml.contains("prst=\"star5\""));
    assert!(xml.contains("prst=\"rightArrow\""));
    assert!(xml.contains("prst=\"chevron\""));
    assert!(xml.contains("schemeClr val=\"accent1\""));
    assert!(xml.contains("schemeClr val=\"accent2\""));
    assert!(xml.contains("lumMod val=\"60000\""));
    assert!(xml.contains("lumOff val=\"40000\""));
    assert!(xml.contains("shade val=\"50000\""));
    assert!(xml.contains("tint val=\"25000\""));

    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse shape preset fixture");
    let shapes = document
        .body
        .as_ref()
        .expect("document body")
        .elements
        .iter()
        .filter_map(|element| element.as_paragraph())
        .flat_map(|paragraph| paragraph.runs.iter())
        .filter_map(|run| run.text_box.as_ref())
        .collect::<Vec<_>>();

    assert_eq!(shapes.len(), 14);
    assert_eq!(shapes[0].shape, jade_docx_reader::TextBoxShape::Ellipse);
    assert!(shapes[0].blocks.is_empty());
    assert_eq!(shapes[0].fill_theme_color.as_deref(), Some("accent1"));
    assert_eq!(
        shapes[0].fill_theme_transforms,
        vec![
            ThemeColorTransform::LuminanceMod(0.6),
            ThemeColorTransform::LuminanceOffset(0.4),
            ThemeColorTransform::Shade(0.5),
            ThemeColorTransform::Tint(0.25),
        ]
    );
    assert_eq!(shapes[0].border.theme_color.as_deref(), Some("accent2"));
    assert_eq!(
        shapes[1].shape,
        jade_docx_reader::TextBoxShape::RoundedRectangle
    );
    assert_eq!(shapes[1].text, "Rounded shape text");
    assert_eq!(shapes[2].shape, jade_docx_reader::TextBoxShape::Diamond);
    assert!(shapes[2].blocks.is_empty());
    assert_eq!(shapes[2].fill_color.as_deref(), Some("#F4B183"));
    assert_eq!(shapes[3].shape, jade_docx_reader::TextBoxShape::Triangle);
    assert!(shapes[3].blocks.is_empty());
    assert_eq!(shapes[3].fill_color.as_deref(), Some("#A9D18E"));
    assert_eq!(
        shapes[4].shape,
        jade_docx_reader::TextBoxShape::RightTriangle
    );
    assert!(shapes[4].blocks.is_empty());
    assert_eq!(shapes[4].fill_color.as_deref(), Some("#9DC3E6"));
    assert_eq!(
        shapes[5].shape,
        jade_docx_reader::TextBoxShape::Parallelogram
    );
    assert!(shapes[5].blocks.is_empty());
    assert_eq!(shapes[5].fill_color.as_deref(), Some("#FFD966"));
    assert_eq!(shapes[6].shape, jade_docx_reader::TextBoxShape::Trapezoid);
    assert!(shapes[6].blocks.is_empty());
    assert_eq!(shapes[6].fill_color.as_deref(), Some("#D5A6BD"));
    assert_eq!(shapes[7].shape, jade_docx_reader::TextBoxShape::Pentagon);
    assert!(shapes[7].blocks.is_empty());
    assert_eq!(shapes[7].fill_color.as_deref(), Some("#C9DAF8"));
    assert_eq!(shapes[8].shape, jade_docx_reader::TextBoxShape::Hexagon);
    assert!(shapes[8].blocks.is_empty());
    assert_eq!(shapes[8].fill_color.as_deref(), Some("#B6D7A8"));
    assert_eq!(shapes[9].shape, jade_docx_reader::TextBoxShape::Octagon);
    assert!(shapes[9].blocks.is_empty());
    assert_eq!(shapes[9].fill_color.as_deref(), Some("#EAD1DC"));
    assert_eq!(shapes[10].shape, jade_docx_reader::TextBoxShape::Plus);
    assert!(shapes[10].blocks.is_empty());
    assert_eq!(shapes[10].fill_color.as_deref(), Some("#FFE699"));
    assert_eq!(
        shapes[11].shape,
        jade_docx_reader::TextBoxShape::FivePointStar
    );
    assert!(shapes[11].blocks.is_empty());
    assert_eq!(shapes[11].fill_color.as_deref(), Some("#F8CBAD"));
    assert_eq!(shapes[12].shape, jade_docx_reader::TextBoxShape::RightArrow);
    assert!(shapes[12].blocks.is_empty());
    assert_eq!(shapes[12].fill_color.as_deref(), Some("#A4C2F4"));
    assert_eq!(shapes[13].shape, jade_docx_reader::TextBoxShape::Chevron);
    assert!(shapes[13].blocks.is_empty());
    assert_eq!(shapes[13].fill_color.as_deref(), Some("#B4A7D6"));
}

#[test]
fn hyperlink_field_fixture_preserves_instrtext_and_interaction_metadata() {
    let xml = read_document_xml("hyperlink-field-edge-cases.docx");
    assert!(xml.contains("HYPERLINK"));
    assert!(
        xml.matches("<w:instrText").count() >= 2,
        "external link instruction remains fragmented across runs"
    );
    assert!(xml.contains("<w:fldSimple"));

    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse hyperlink field fixture");
    let body = document.body.as_ref().expect("document body present");
    let links: Vec<_> = body
        .elements
        .iter()
        .filter_map(jade_docx_reader::BodyElement::as_paragraph)
        .flat_map(|paragraph| paragraph.runs.iter())
        .filter_map(|run| run.hyperlink.as_ref())
        .collect();
    assert_eq!(links.len(), 2);
    assert!(links.iter().any(|link| {
        link.target.as_deref() == Some("https://example.com/a b")
            && link.anchor.as_deref() == Some("details")
            && link.tooltip.as_deref() == Some("Open details")
            && link.target_frame.as_deref() == Some("ReportFrame")
    }));
    assert!(links
        .iter()
        .any(|link| { link.target.is_none() && link.anchor.as_deref() == Some("local target") }));
    let destination = document
        .bookmarks
        .iter()
        .find(|bookmark| bookmark.name == "local target")
        .expect("local destination bookmark");
    assert_eq!(destination.start_offset, 0);
    assert_eq!(destination.end_offset, "Local destination".len());
}

#[test]
fn image_crop_fixture_preserves_drawingml_source_rectangle() {
    let xml = read_document_xml("image-crop-edge-cases.docx");
    assert!(xml.contains("<a:srcRect l=\"15000\" t=\"5000\" r=\"25000\" b=\"10000\""));

    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse cropped-image fixture");
    let image = document
        .body
        .as_ref()
        .expect("document body")
        .elements
        .iter()
        .filter_map(|element| element.as_paragraph())
        .flat_map(|paragraph| paragraph.runs.iter())
        .find_map(|run| run.image.as_ref())
        .expect("inline image");
    let crop = image.crop.expect("source crop metadata");
    assert_eq!((crop.left, crop.top), (0.15, 0.05));
    assert_eq!((crop.right, crop.bottom), (0.25, 0.10));
}

/// `yokohama_table_row_heights.docx` is a 6-column table Word 365 inserted into
/// a yokohama-style document and saved as-is: no `w:tblStyle`, no direct
/// `w:tblCellMar`, three auto rows and two `hRule="exact"` rows. Word for Mac
/// (AppleScript `get range information`, 2026-09-03) lays the rows out at
/// 15.35 / 14.55 / 46.2 / 5.0 / 14.55 pt, so the exact rows keep their
/// declared height and the auto rows are one empty 10.5pt 游明朝 line.
#[test]
fn yokohama_table_row_heights_fixture_keeps_word_row_rules() {
    let xml = read_document_xml("yokohama_table_row_heights.docx");
    assert!(xml.contains("<w:trHeight w:val=\"924\" w:hRule=\"exact\"/>"));
    assert!(xml.contains("<w:trHeight w:val=\"100\" w:hRule=\"exact\"/>"));
    assert!(
        !xml.contains("<w:tblStyle "),
        "the fixture table must rely on the default table style"
    );
    assert!(
        !xml.contains("<w:tblCellMar"),
        "the fixture table must rely on the default cell margins"
    );
    assert!(
        !xml.contains("<w:docGrid"),
        "the fixture section has no document grid"
    );

    let document = DocumentParser::new()
        .parse_document_xml(&xml)
        .expect("parse yokohama table row-height fixture");
    let table = document
        .body
        .as_ref()
        .into_iter()
        .flat_map(|body| &body.elements)
        .find_map(|element| match element {
            jade_docx_reader::BodyElement::Table(table) => Some(table.as_ref()),
            _ => None,
        })
        .expect("fixture table");

    assert_eq!(table.rows.len(), 5);
    assert!(table.style_name.is_none());
    assert!(table.cell_margins.is_none());
    let heights: Vec<_> = table
        .rows
        .iter()
        .map(|row| (row.height, row.height_rule))
        .collect();
    assert_eq!(
        heights,
        vec![
            (None, None),
            (None, None),
            (Some(46.2), Some(TableRowHeightRule::Exact)),
            (Some(5.0), Some(TableRowHeightRule::Exact)),
            (None, None),
        ]
    );
    assert!(table
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .all(|cell| cell.margins.is_none()));
}

#[test]
fn character_styles_fixture_lists_gallery_styles_and_hides_hyperlink() {
    let xml = read_part_xml("character-styles.docx", "word/styles.xml");
    let styles = jade_docx_reader::styles::StylesPart::from_xml(&xml).expect("parse styles");

    // The Storybook `CharacterStyles` story needs a non-empty gallery: the
    // toolbar renders no 文字スタイル select for an empty one.
    let mut gallery: Vec<&str> = styles
        .character_style_gallery_definitions()
        .map(|definition| definition.style_id.as_str())
        .collect();
    gallery.sort_unstable();
    assert_eq!(gallery, ["Emphasis", "Strong"]);

    // `Hyperlink` is hidden from the gallery but still defined, so applying it
    // by id must keep resolving.
    assert_eq!(
        styles.resolve_character_style_id("Hyperlink"),
        Some("Hyperlink")
    );
}
