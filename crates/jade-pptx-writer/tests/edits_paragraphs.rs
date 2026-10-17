use jade_opc::OpcPackage;
use jade_pptx_reader::ShapeItem;
use jade_pptx_writer::{
    format_paragraphs, shape_text, EditSession, ParagraphAlignment, ParagraphBullet,
    ParagraphFormat, ParagraphFormatEdit, ParagraphSpacing,
};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn custom(body: &str, locks: &str) -> Vec<u8> {
    let mut package = OpcPackage::open(DECK).unwrap();
    package.replace("ppt/slides/slide1.xml", format!(r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="7" name="Text box"/><p:cNvSpPr txBox="1">{locks}</p:cNvSpPr><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="9144000" cy="4572000"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/>{body}</p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#).into_bytes());
    package.write().unwrap()
}
fn edit(start: usize, end: usize, format: ParagraphFormat) -> ParagraphFormatEdit {
    ParagraphFormatEdit {
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
fn caret_changes_only_its_paragraph_and_preserves_text_properties_and_parts() {
    let bytes = custom(
        r#"<a:p><a:pPr algn='l' rtl='0'><a:tabLst><a:tab pos="12700" algn="l"/></a:tabLst><a:defRPr b="1"/><a:extLst><a:ext uri="x"><z:opaque xmlns:z="urn:test"/></a:ext></a:extLst></a:pPr><a:r><a:rPr i="1"/><a:t>abcd</a:t></a:r><a:br/><a:r><a:t>x</a:t></a:r></a:p><a:p><a:r><a:t>next</a:t></a:r></a:p>"#,
        "",
    );
    let format = ParagraphFormat {
        alignment: Some(ParagraphAlignment::Center),
        level: Some(2),
        margin_left_pt: Some(36.0),
        margin_right_pt: None,
        indent_pt: Some(-18.0),
        line_spacing: Some(ParagraphSpacing::Percent(150.0)),
        space_before: Some(ParagraphSpacing::Points(6.0)),
        space_after: Some(ParagraphSpacing::Points(12.0)),
        bullet: Some(ParagraphBullet::Character {
            character: "•".into(),
            font: None,
        }),
    };
    let edited = format_paragraphs(&bytes, 0, &edit(5, 5, format.clone())).unwrap();
    assert_eq!(shape_text(&edited, 0, 7).unwrap().text, "abcd\nx\nnext");
    let p = paragraphs(&edited);
    assert_eq!(p[0].align, Some(jade_pptx_reader::TextAlign::Center));
    assert_eq!(
        (p[0].level, p[0].margin_left_emu, p[0].indent_emu),
        (2, Some(457200), Some(-228600))
    );
    assert!(p[0].bullet_font_follow_text);
    assert!(p[0].runs[0].italic);
    assert_eq!(p[1].align, None);
    let result = xml(&edited);
    assert!(result.contains("rtl='0'"));
    assert!(result.contains("<z:opaque xmlns:z=\"urn:test\"/>"));
    let start = result.find("<jadePara:lnSpc").unwrap();
    let tail = &result[start..];
    for (before, after) in [
        ("lnSpc", "spcBef"),
        ("spcBef", "spcAft"),
        ("spcAft", "buFontTx"),
        ("buFontTx", "buChar"),
        ("buChar", "tabLst"),
    ] {
        assert!(tail.find(before).unwrap() < tail.find(after).unwrap());
    }
    let original = OpcPackage::open(&bytes).unwrap();
    let changed = OpcPackage::open(&edited).unwrap();
    for part in original.part_names() {
        if part != "ppt/slides/slide1.xml" {
            assert_eq!(original.part(part), changed.part(part), "{part}");
        }
    }
    assert_eq!(
        format_paragraphs(&edited, 0, &edit(5, 5, format)).unwrap(),
        edited
    );
}

#[test]
fn empty_paragraphs_and_selection_ending_at_next_start_have_consistent_scope() {
    let bytes = custom(
        "<a:p><a:r><a:t>a</a:t></a:r></a:p><a:p/><a:p><a:r><a:t>b</a:t></a:r></a:p><a:p/>",
        "",
    );
    let format = ParagraphFormat {
        alignment: Some(ParagraphAlignment::Right),
        ..Default::default()
    };
    let edited = format_paragraphs(&bytes, 0, &edit(0, 3, format.clone())).unwrap();
    let p = paragraphs(&edited);
    assert_eq!(
        p.iter().map(|p| p.align).collect::<Vec<_>>(),
        [
            Some(jade_pptx_reader::TextAlign::Right),
            Some(jade_pptx_reader::TextAlign::Right),
            None,
            None
        ]
    );
    let edited = format_paragraphs(&bytes, 0, &edit(5, 5, format)).unwrap();
    assert_eq!(
        paragraphs(&edited)[3].align,
        Some(jade_pptx_reader::TextAlign::Right)
    );
}

#[test]
fn numbering_restarts_only_the_first_selected_paragraph_and_new_paragraphs_continue() {
    let bytes = custom("<a:p><a:r><a:t>a</a:t></a:r></a:p><a:p><a:r><a:t>b</a:t></a:r></a:p><a:p><a:r><a:t>c</a:t></a:r></a:p>", "");
    let mut session = EditSession::open(&bytes).unwrap();
    let format = ParagraphFormat {
        bullet: Some(ParagraphBullet::Numbered {
            scheme: "arabicPeriod".into(),
            start_at: Some(5),
        }),
        ..Default::default()
    };
    session
        .format_paragraphs(0, &edit(0, 5, format.clone()))
        .unwrap();
    let numbered = session.bytes().to_vec();
    let p = paragraphs(&numbered);
    assert_eq!(
        p.iter().map(|p| p.number_start).collect::<Vec<_>>(),
        [Some(5), None, None]
    );
    assert!(!session.format_paragraphs(0, &edit(0, 5, format)).unwrap());
    session.set_shape_text(0, 7, "a\nx\nb\nc", false).unwrap();
    let p = paragraphs(session.bytes());
    assert_eq!(
        p.iter().map(|p| p.number_start).collect::<Vec<_>>(),
        [Some(5), None, None, None]
    );
    let deck = jade_pptx_reader::read_deck(session.bytes()).unwrap();
    let layout =
        jade_pptx_layout::layout_deck_slide(&deck, 0, &jade_pptx_layout::HeuristicMeasure).unwrap();
    let markers: Vec<_> = layout
        .shapes
        .iter()
        .find(|s| s.source_id == Some(7))
        .unwrap()
        .lines
        .iter()
        .flat_map(|l| &l.runs)
        .filter(|r| r.source_offsets.is_empty())
        .map(|r| r.text.as_str())
        .collect();
    assert_eq!(markers, ["5.", "6.", "7.", "8."]);
    assert!(session.undo());
    assert_eq!(session.bytes(), numbered);
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
    assert!(session.redo());
    assert_eq!(session.bytes(), numbered);
}

#[test]
fn switching_list_kinds_preserves_metadata_and_spacing_mode_changes() {
    let bytes = custom(
        r#"<a:p><a:pPr><a:lnSpc><a:spcPct val="100000"/></a:lnSpc><a:buClr><a:srgbClr val="123456"/></a:buClr><a:buSzPct val="80000"/><a:buFont typeface='Wingdings'/><a:buAutoNum type="arabicPeriod" startAt='7' z:keep="yes" xmlns:z="urn:test"/></a:pPr><a:r><a:t>x</a:t></a:r></a:p>"#,
        "",
    );
    let numbered = format_paragraphs(
        &bytes,
        0,
        &edit(
            0,
            0,
            ParagraphFormat {
                bullet: Some(ParagraphBullet::Numbered {
                    scheme: "romanUcPeriod".into(),
                    start_at: None,
                }),
                line_spacing: Some(ParagraphSpacing::Points(24.5)),
                ..Default::default()
            },
        ),
    )
    .unwrap();
    let result = xml(&numbered);
    assert!(!result.contains("startAt="));
    assert!(result.contains("z:keep=\"yes\""));
    let p = &paragraphs(&numbered)[0];
    assert_eq!(
        p.line_spacing,
        Some(jade_pptx_reader::Spacing::Points(24.5))
    );
    assert!(p.bullet_font_follow_text);
    let none = format_paragraphs(
        &numbered,
        0,
        &edit(
            0,
            0,
            ParagraphFormat {
                bullet: Some(ParagraphBullet::None),
                ..Default::default()
            },
        ),
    )
    .unwrap();
    assert_eq!(
        paragraphs(&none)[0].bullet,
        Some(jade_pptx_reader::Bullet::None)
    );
    assert!(xml(&none).contains("<a:buClr>"));
    assert!(xml(&none).contains("<a:buSzPct val=\"80000\"/>"));
}

#[test]
fn paragraph_validation_is_transactional_for_ranges_locks_fields_and_invalid_values() {
    let bytes = custom("<a:p><a:r><a:t>🦀x</a:t></a:r></a:p>", "");
    let mut session = EditSession::open(&bytes).unwrap();
    for format in [
        ParagraphFormat {
            level: Some(9),
            ..Default::default()
        },
        ParagraphFormat {
            margin_left_pt: Some(-1.0),
            ..Default::default()
        },
        ParagraphFormat {
            indent_pt: Some(f64::INFINITY),
            ..Default::default()
        },
        ParagraphFormat {
            line_spacing: Some(ParagraphSpacing::Percent(0.0)),
            ..Default::default()
        },
        ParagraphFormat {
            space_after: Some(ParagraphSpacing::Points(1584.01)),
            ..Default::default()
        },
        ParagraphFormat {
            bullet: Some(ParagraphBullet::Numbered {
                scheme: "invalid".into(),
                start_at: None,
            }),
            ..Default::default()
        },
        ParagraphFormat {
            bullet: Some(ParagraphBullet::Character {
                character: "two".into(),
                font: None,
            }),
            ..Default::default()
        },
    ] {
        assert!(session.format_paragraphs(0, &edit(0, 0, format)).is_err());
    }
    let format = ParagraphFormat {
        alignment: Some(ParagraphAlignment::Center),
        ..Default::default()
    };
    for (start, end) in [(1, 1), (3, 0), (0, 4)] {
        assert!(session
            .format_paragraphs(0, &edit(start, end, format.clone()))
            .is_err());
    }
    assert_eq!(session.bytes(), bytes);
    assert!(!session.can_undo());
    for (body, locks) in [
        (
            "<a:p><a:r><a:t>x</a:t></a:r></a:p>",
            "<a:spLocks noTextEdit=\"1\"/>",
        ),
        ("<a:p><a:fld id=\"field\"><a:t>x</a:t></a:fld></a:p>", ""),
    ] {
        assert!(format_paragraphs(&custom(body, locks), 0, &edit(0, 0, format.clone())).is_err());
    }
}

#[test]
fn strict_namespaces_and_quoted_bullet_fonts_survive_save_and_reapplication() {
    let bytes = custom("<a:p><a:pPr/><a:r><a:t>x</a:t></a:r></a:p>", "");
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
    let bytes = package.write().unwrap();
    let format = ParagraphFormat {
        bullet: Some(ParagraphBullet::Character {
            character: "&".into(),
            font: Some("A & B 'face'".into()),
        }),
        space_before: Some(ParagraphSpacing::Points(0.0)),
        ..Default::default()
    };
    let edited = format_paragraphs(&bytes, 0, &edit(0, 1, format.clone())).unwrap();
    assert_eq!(
        paragraphs(&edited)[0].bullet_font.as_deref(),
        Some("A & B 'face'")
    );
    assert_eq!(
        paragraphs(&edited)[0].bullet,
        Some(jade_pptx_reader::Bullet::Char("&".into()))
    );
    assert_eq!(
        format_paragraphs(&edited, 0, &edit(0, 1, format)).unwrap(),
        edited
    );
}

#[test]
fn justification_distributed_and_right_margin_preserve_runs_extensions_and_exact_history() {
    let bytes = custom(
        r#"<a:p><a:pPr algn="l" marR="12700"><a:buNone/><a:extLst><a:ext uri="test"><q:data xmlns:q="urn:test"/></a:ext></a:extLst></a:pPr><a:r><a:rPr b="1"/><a:t>one two three</a:t></a:r></a:p>"#,
        "",
    );
    for (alignment, parsed, token) in [
        (
            ParagraphAlignment::Justify,
            jade_pptx_reader::TextAlign::Justify,
            "just",
        ),
        (
            ParagraphAlignment::Distributed,
            jade_pptx_reader::TextAlign::Distributed,
            "dist",
        ),
    ] {
        let mut s = EditSession::open(&bytes).unwrap();
        let edit = edit(
            0,
            13,
            ParagraphFormat {
                alignment: Some(alignment),
                margin_right_pt: Some(18.5),
                ..Default::default()
            },
        );
        assert!(s.format_paragraphs(0, &edit).unwrap());
        let saved = s.bytes().to_vec();
        let p = paragraphs(&saved);
        assert_eq!(p[0].align, Some(parsed));
        assert_eq!(p[0].margin_right_emu, Some(234950));
        assert_eq!(p[0].runs, paragraphs(&bytes)[0].runs);
        let p = OpcPackage::open(&saved).unwrap();
        let xml = p.required_part_str("ppt/slides/slide1.xml").unwrap();
        assert!(xml.contains(&format!("algn=\"{token}\"")));
        assert!(xml.contains("<q:data xmlns:q=\"urn:test\"/>"));
        let original = OpcPackage::open(&bytes).unwrap();
        for part in original
            .part_names()
            .iter()
            .filter(|p| p.as_str() != "ppt/slides/slide1.xml")
        {
            assert_eq!(original.part(part), p.part(part));
        }
        assert!(!s.format_paragraphs(0, &edit).unwrap());
        assert!(s.undo());
        assert_eq!(s.bytes(), bytes);
        assert!(!s.undo());
        assert!(s.redo());
        assert_eq!(s.bytes(), saved);
    }
}
#[test]
fn right_margin_rejects_invalid_values_without_mutation_and_accepts_drawingml_bounds() {
    let bytes = custom("<a:p><a:r><a:t>text</a:t></a:r></a:p>", "");
    let mut s = EditSession::open(&bytes).unwrap();
    for value in [-1.0, 4032.001, f64::NAN, f64::INFINITY] {
        assert!(s
            .format_paragraphs(
                0,
                &edit(
                    0,
                    4,
                    ParagraphFormat {
                        margin_right_pt: Some(value),
                        ..Default::default()
                    }
                )
            )
            .is_err());
    }
    assert_eq!(s.bytes(), bytes);
    assert!(!s.undo());
    for value in [0.0, 4032.0] {
        s.format_paragraphs(
            0,
            &edit(
                0,
                4,
                ParagraphFormat {
                    margin_right_pt: Some(value),
                    ..Default::default()
                },
            ),
        )
        .unwrap();
        assert_eq!(
            paragraphs(s.bytes())[0].margin_right_emu,
            Some((value * 12700.0) as i64)
        );
    }
}
