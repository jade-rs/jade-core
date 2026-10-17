use jade_opc::OpcPackage;
use jade_pptx_reader::ShapeItem;
use jade_pptx_writer::{format_text_range, shape_text, EditSession, TextFormat, TextFormatEdit};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn custom(body: &str, locks: &str) -> Vec<u8> {
    let mut package = OpcPackage::open(DECK).unwrap();
    package.replace("ppt/slides/slide1.xml", format!(r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="7" name="Text box"/><p:cNvSpPr txBox="1">{locks}</p:cNvSpPr><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="9144000" cy="4572000"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/>{body}</p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#).into_bytes());
    package.write().unwrap()
}
fn edit(start: usize, end: usize, format: TextFormat) -> TextFormatEdit {
    TextFormatEdit {
        shape_id: 7,
        start,
        end,
        format,
    }
}
fn paragraphs(bytes: &[u8]) -> Vec<jade_pptx_reader::TextParagraph> {
    let deck = jade_pptx_reader::read_deck(bytes).unwrap();
    let ShapeItem::Shape(shape) = &deck.slides[0].content.shapes[0] else {
        panic!("shape")
    };
    shape.text.clone().unwrap().paragraphs
}
fn xml(bytes: &[u8]) -> String {
    OpcPackage::open(bytes)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap()
}

#[test]
fn partial_format_splits_only_selected_text_preserving_links_extensions_and_parts() {
    let bytes = custom(
        r#"<a:p><a:pPr algn="ctr"/><a:r lang="ja-JP"><a:rPr b='true' i="0" u="none" sz="1800"><a:solidFill><a:srgbClr val="123456"/></a:solidFill><a:latin typeface="Aptos"/><a:hlinkClick r:id="rId99"/><a:extLst><a:ext uri="test"><z:data xmlns:z="urn:test"/></a:ext></a:extLst></a:rPr><a:t>日本語🦀ABC</a:t></a:r><a:endParaRPr sz="2200"/></a:p>"#,
        "",
    );
    let format = TextFormat {
        italic: Some(true),
        underline: Some(true),
        size_pt: Some(36.25),
        ..Default::default()
    };
    let edited = format_text_range(&bytes, 0, &edit(1, 5, format.clone())).unwrap();
    assert_eq!(shape_text(&edited, 0, 7).unwrap().text, "日本語🦀ABC");
    let runs = paragraphs(&edited).remove(0).runs;
    assert_eq!(
        runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
        ["日", "本語🦀", "ABC"]
    );
    assert!(runs[0].bold && !runs[0].italic && runs[0].size_pt == Some(18.0));
    assert!(runs[1].italic && runs[1].underline && runs[1].size_pt == Some(36.25));
    assert!(runs[2].bold && !runs[2].italic && runs[2].size_pt == Some(18.0));
    assert!(runs
        .iter()
        .all(|r| r.hyperlink.is_some() && r.font.as_deref() == Some("Aptos")));
    assert_eq!(
        xml(&edited)
            .matches("<z:data xmlns:z=\"urn:test\"/>")
            .count(),
        3
    );
    assert_eq!(xml(&edited).matches("b='true'").count(), 3);
    let before = OpcPackage::open(&bytes).unwrap();
    let after = OpcPackage::open(&edited).unwrap();
    for part in before.part_names() {
        if part != "ppt/slides/slide1.xml" {
            assert_eq!(before.part(part), after.part(part), "{part}");
        }
    }
    assert_eq!(
        format_text_range(&edited, 0, &edit(1, 5, format)).unwrap(),
        edited
    );
}

#[test]
fn spans_paragraph_marks_empty_paragraphs_and_formatted_soft_breaks() {
    let bytes = custom("<a:p><a:r><a:t>ab</a:t></a:r></a:p><a:p/><a:p><a:r><a:t>c</a:t></a:r><a:br/><a:r><a:t>d</a:t></a:r></a:p>", "");
    let edited = format_text_range(
        &bytes,
        0,
        &edit(
            1,
            6,
            TextFormat {
                bold: Some(true),
                size_pt: Some(48.0),
                ..Default::default()
            },
        ),
    )
    .unwrap();
    assert_eq!(shape_text(&edited, 0, 7).unwrap().text, "ab\n\nc\nd");
    let p = paragraphs(&edited);
    assert_eq!(p[0].end_run_props.bold, Some(true));
    assert_eq!(p[1].end_run_props.bold, Some(true));
    assert!(p[2].runs[0].bold && p[2].runs[1].bold);
    assert_eq!(p[2].runs[1].text, "\n");
    assert_eq!(p[2].runs[1].size_pt, Some(48.0));
    assert!(!p[2].runs[2].bold);
    assert_eq!(
        format_text_range(
            &edited,
            0,
            &edit(
                1,
                6,
                TextFormat {
                    bold: Some(true),
                    size_pt: Some(48.0),
                    ..Default::default()
                }
            )
        )
        .unwrap(),
        edited
    );
}

#[test]
fn sizes_ranges_locks_and_fields_are_validated_transactionally() {
    let bytes = custom("<a:p><a:r><a:t>A🦀B</a:t></a:r></a:p>", "");
    let mut session = EditSession::open(&bytes).unwrap();
    for size in [0.0, 0.99, 4000.01, f64::INFINITY, f64::NAN] {
        assert!(session
            .format_text_range(
                0,
                &edit(
                    0,
                    1,
                    TextFormat {
                        size_pt: Some(size),
                        ..Default::default()
                    }
                )
            )
            .is_err());
    }
    for (start, end) in [(2, 3), (1, 2), (3, 1), (0, 8)] {
        assert!(session
            .format_text_range(
                0,
                &edit(
                    start,
                    end,
                    TextFormat {
                        bold: Some(true),
                        ..Default::default()
                    }
                )
            )
            .is_err());
    }
    assert_eq!(session.bytes(), bytes);
    assert!(!session.can_undo());
    for (body, locks) in [
        (
            "<a:p><a:r><a:t>text</a:t></a:r></a:p>",
            "<a:spLocks noTextEdit=\"1\"/>",
        ),
        ("<a:p><a:fld id=\"field\"><a:t>x</a:t></a:fld></a:p>", ""),
    ] {
        assert!(format_text_range(
            &custom(body, locks),
            0,
            &edit(
                0,
                1,
                TextFormat {
                    bold: Some(true),
                    ..Default::default()
                }
            )
        )
        .is_err());
    }
    assert_eq!(
        format_text_range(
            &bytes,
            0,
            &edit(
                1,
                1,
                TextFormat {
                    bold: Some(true),
                    ..Default::default()
                }
            )
        )
        .unwrap(),
        bytes
    );
    assert_eq!(
        format_text_range(&bytes, 0, &edit(0, 4, TextFormat::default())).unwrap(),
        bytes
    );
}

#[test]
fn empty_paragraph_formatting_is_saved_and_inherited_by_future_typing() {
    let bytes = custom("<a:p><a:endParaRPr i=\"1\"/></a:p>", "");
    let mut session = EditSession::open(&bytes).unwrap();
    session
        .format_text_range(
            0,
            &edit(
                0,
                0,
                TextFormat {
                    bold: Some(true),
                    size_pt: Some(24.0),
                    ..Default::default()
                },
            ),
        )
        .unwrap();
    session.set_shape_text(0, 7, "text", false).unwrap();
    let run = paragraphs(session.bytes())
        .remove(0)
        .runs
        .into_iter()
        .find(|r| r.text == "text")
        .unwrap();
    assert!(run.bold && run.italic && run.size_pt == Some(24.0));
    assert!(session.undo());
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
}

#[test]
fn formatted_typing_changes_only_inserted_splice_in_one_history_entry() {
    let bytes = custom("<a:p><a:r><a:rPr i=\"1\"/><a:t>ABC</a:t></a:r></a:p>", "");
    let mut session = EditSession::open(&bytes).unwrap();
    session
        .set_shape_text_formatted(
            0,
            7,
            "A日本\n次BC",
            false,
            &TextFormat {
                bold: Some(true),
                underline: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    let p = paragraphs(session.bytes());
    assert!(p[0]
        .runs
        .iter()
        .filter(|r| r.text == "日本")
        .all(|r| r.bold && r.italic && r.underline));
    assert!(p[1]
        .runs
        .iter()
        .filter(|r| r.text == "次")
        .all(|r| r.bold && r.italic && r.underline));
    assert!(p
        .iter()
        .flat_map(|p| &p.runs)
        .filter(|r| r.text == "A" || r.text == "BC")
        .all(|r| !r.bold));
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
    assert!(!session.can_undo());
    assert!(session.redo());
}

#[test]
fn alternate_and_strict_namespaces_keep_prefix_scopes_during_splits() {
    let bytes = custom(
        r#"<a:p xmlns:z="http://schemas.openxmlformats.org/drawingml/2006/main"><z:r><z:rPr u="sng"/><z:t>abcd</z:t></z:r></a:p>"#,
        "",
    );
    let changed = format_text_range(
        &bytes,
        0,
        &edit(
            1,
            3,
            TextFormat {
                underline: Some(false),
                ..Default::default()
            },
        ),
    )
    .unwrap();
    assert_eq!(
        paragraphs(&changed)[0].runs[1].underline_stated,
        Some(false)
    );
    assert!(xml(&changed).contains("<z:rPr u=\"none\""));
    let mut package = OpcPackage::open(&bytes).unwrap();
    package.replace(
        "ppt/slides/slide1.xml",
        xml(&bytes)
            .replace(
                "http://schemas.openxmlformats.org/drawingml/2006/main",
                "http://purl.oclc.org/ooxml/drawingml/main",
            )
            .replace(
                "http://schemas.openxmlformats.org/presentationml/2006/main",
                "http://purl.oclc.org/ooxml/presentationml/main",
            )
            .into_bytes(),
    );
    let strict = package.write().unwrap();
    let changed = format_text_range(
        &strict,
        0,
        &edit(
            1,
            3,
            TextFormat {
                size_pt: Some(14.0),
                font: Some("01".into()),
                east_asian_font: Some("游ゴシック".into()),
                color: Some("#00FF88".into()),
                ..Default::default()
            },
        ),
    )
    .unwrap();
    let run = &paragraphs(&changed)[0].runs[1];
    assert_eq!(run.size_pt, Some(14.0));
    assert_eq!(run.font.as_deref(), Some("01"));
    assert_eq!(run.east_asian_font.as_deref(), Some("游ゴシック"));
    assert_eq!(
        run.color,
        Some(jade_pptx_reader::Color::Srgb("00FF88".into()))
    );
}

#[test]
fn replacing_a_selection_at_a_run_boundary_inherits_the_selected_run() {
    let bytes = custom(
        "<a:p><a:r><a:t>left</a:t></a:r><a:r><a:rPr b=\"1\"/><a:t>right</a:t></a:r></a:p>",
        "",
    );
    let edited = jade_pptx_writer::replace_text_range(&bytes, 0, 7, 4, 9, "NEW", false).unwrap();
    assert!(
        paragraphs(&edited)[0]
            .runs
            .iter()
            .find(|r| r.text == "NEW")
            .unwrap()
            .bold
    );
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(80))]
    #[test]
    fn formatting_unicode_ranges_keeps_text_and_unselected_properties(
        chars in proptest::collection::vec(proptest::sample::select(vec!['a', '日', '🦀', '&', '<', '\n', ' ', '\u{3099}']), 0..30),
        a in 0usize..40, b in 0usize..40,
    ) {
        let plain: String = chars.iter().collect();
        let body: String = plain.split('\n').map(|line| format!("<a:p><a:r><a:rPr b=\"0\"/><a:t>{}</a:t></a:r></a:p>", quick_xml::escape::escape(line))).collect();
        let bytes = custom(&body, "");
        let a = a % (chars.len() + 1); let b = b % (chars.len() + 1);
        let start = chars[..a.min(b)].iter().map(|c| c.len_utf16()).sum();
        let end = chars[..a.max(b)].iter().map(|c| c.len_utf16()).sum();
        let edited = format_text_range(&bytes, 0, &edit(start, end, TextFormat { bold: Some(true), ..Default::default() })).unwrap();
        proptest::prop_assert_eq!(shape_text(&edited, 0, 7).unwrap().text, plain);
        let mut offset = 0;
        for p in paragraphs(&edited) {
            for run in p.runs {
                for ch in run.text.chars() {
                    proptest::prop_assert_eq!(run.bold, offset >= start && offset < end);
                    offset += ch.len_utf16();
                }
            }
            offset += 1;
        }
    }
}

#[test]
fn font_color_partial_edits_preserve_unselected_styles_and_schema_order() {
    let bytes = custom(
        r#"<a:p><a:r><a:rPr><a:ln w="12700"/><a:gradFill><a:gsLst/></a:gradFill><a:effectLst/><a:latin typeface='Old' charset='1'/><a:hlinkClick r:id="rId99"/><a:extLst/></a:rPr><a:t>abcd</a:t></a:r></a:p>"#,
        "",
    );
    let format = TextFormat {
        font: Some("A & B \"quoted\" 'face'".into()),
        east_asian_font: Some("游ゴシック".into()),
        color: Some("#12abEF".into()),
        ..Default::default()
    };
    let edited = format_text_range(&bytes, 0, &edit(1, 3, format.clone())).unwrap();
    let runs = paragraphs(&edited).remove(0).runs;
    assert_eq!(
        runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
        ["a", "bc", "d"]
    );
    assert_eq!(runs[1].font, format.font);
    assert_eq!(runs[1].east_asian_font, format.east_asian_font);
    assert_eq!(
        runs[1].color,
        Some(jade_pptx_reader::Color::Srgb("12ABEF".into()))
    );
    assert_eq!(runs[0].font.as_deref(), Some("Old"));
    assert_eq!(runs[2].font.as_deref(), Some("Old"));
    let xml = xml(&edited);
    assert_eq!(xml.matches("charset='1'").count(), 3);
    assert_eq!(xml.matches("<a:hlinkClick r:id=\"rId99\"/>").count(), 3);
    let fill = xml.find("<jadeFmt:solidFill").unwrap();
    let after = &xml[fill..];
    assert!(after.find("<a:effectLst").unwrap() < after.find("<a:latin").unwrap());
    assert!(after.find("<a:latin").unwrap() < after.find("<jadeFmt:ea").unwrap());
    assert!(after.find("<jadeFmt:ea").unwrap() < after.find("<a:hlinkClick").unwrap());
    assert_eq!(
        format_text_range(&edited, 0, &edit(1, 3, format)).unwrap(),
        edited
    );
}

#[test]
fn fonts_colors_expand_empty_properties_and_apply_to_marks_and_breaks() {
    let format = TextFormat {
        font: Some("Aptos".into()),
        east_asian_font: Some("游ゴシック".into()),
        color: Some("#0088FF".into()),
        ..Default::default()
    };
    for body in [
        "<a:p><a:r><a:rPr/><a:t>x</a:t></a:r><a:br/></a:p><a:p/>",
        "<a:p><a:r><a:t>x</a:t></a:r><a:br/></a:p><a:p/>",
    ] {
        let bytes = custom(body, "");
        let edited = format_text_range(&bytes, 0, &edit(0, 3, format.clone())).unwrap();
        let p = paragraphs(&edited);
        assert_eq!(p[0].runs[0].font, format.font);
        assert_eq!(
            p[0].runs[1].color,
            Some(jade_pptx_reader::Color::Srgb("0088FF".into()))
        );
        assert_eq!(p[1].end_run_props.east_asian_font, format.east_asian_font);
        assert_eq!(
            format_text_range(&edited, 0, &edit(0, 3, format.clone())).unwrap(),
            edited
        );
    }
}

#[test]
fn invalid_fonts_colors_reject_without_history_and_typing_retains_format() {
    let bytes = custom("<a:p><a:r><a:t>x</a:t></a:r></a:p>", "");
    let mut session = EditSession::open(&bytes).unwrap();
    for format in [
        TextFormat {
            font: Some("".into()),
            ..Default::default()
        },
        TextFormat {
            east_asian_font: Some("x\n".into()),
            ..Default::default()
        },
        TextFormat {
            color: Some("#GG0000".into()),
            ..Default::default()
        },
        TextFormat {
            color: Some("#123".into()),
            ..Default::default()
        },
    ] {
        assert!(session.format_text_range(0, &edit(0, 1, format)).is_err());
        assert_eq!(session.bytes(), bytes);
        assert!(!session.can_undo());
    }
    session
        .set_shape_text_formatted(
            0,
            7,
            "x日",
            false,
            &TextFormat {
                font: Some("Arial".into()),
                east_asian_font: Some("游ゴシック".into()),
                color: Some("#FF8800".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let runs = paragraphs(session.bytes()).remove(0).runs;
    assert_eq!(runs[1].font.as_deref(), Some("Arial"));
    assert_eq!(
        runs[1].color,
        Some(jade_pptx_reader::Color::Srgb("FF8800".into()))
    );
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
}

#[test]
fn numeric_typeface_names_remain_distinct_and_escaped_properties_are_idempotent() {
    let bytes = custom(
        "<a:p><a:r><a:rPr><a:latin typeface=\"1\"/></a:rPr><a:t>x</a:t></a:r></a:p>",
        "",
    );
    let format = TextFormat {
        font: Some("01".into()),
        ..Default::default()
    };
    let edited = format_text_range(&bytes, 0, &edit(0, 1, format.clone())).unwrap();
    assert_eq!(paragraphs(&edited)[0].runs[0].font.as_deref(), Some("01"));
    assert_eq!(
        format_text_range(&edited, 0, &edit(0, 1, format)).unwrap(),
        edited
    );
}
