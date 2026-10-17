use jade_opc::OpcPackage;
use jade_pptx_reader::ShapeItem;
use jade_pptx_writer::{
    editable_runs, replace_run_text, replace_text_range, shape_text, EditSession, TextRangeEdit,
};

const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn custom(content: &str, locks: &str) -> Vec<u8> {
    let mut package = OpcPackage::open(DECK).unwrap();
    package.replace("ppt/slides/slide1.xml", format!(r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="7" name="Text box"/><p:cNvSpPr txBox="1">{locks}</p:cNvSpPr><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="9144000" cy="4572000"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/>{content}</p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#).into_bytes());
    package.write().unwrap()
}
fn text_shape(bytes: &[u8]) -> jade_pptx_reader::Shape {
    let deck = jade_pptx_reader::read_deck(bytes).unwrap();
    let ShapeItem::Shape(shape) = &deck.slides[0].content.shapes[0] else {
        panic!("shape");
    };
    shape.clone()
}
fn p(text: &str) -> String {
    format!(
        "<a:p><a:r><a:t>{}</a:t></a:r></a:p>",
        quick_xml::escape::escape(text)
    )
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(80))]
    #[test]
    fn unicode_splices_match_plain_text_without_losing_package_parts(
        before in proptest::collection::vec(proptest::sample::select(vec!['a', '日', '🦀', ' ', '\n', '\u{3099}', '&', '<']), 0..25),
        insertion in proptest::collection::vec(proptest::sample::select(vec!['b', '本', '😀', '\n', '&']), 0..12),
        a in 0usize..30, b in 0usize..30,
    ) {
        let length = before.len();
        let a = a % (length + 1);
        let b = b % (length + 1);
        let (start_char, end_char) = (a.min(b), a.max(b));
        let plain: String = before.iter().collect();
        let body: String = plain.split('\n').map(p).collect();
        let bytes = custom(&body, "");
        let start = before[..start_char].iter().map(|c| c.len_utf16()).sum();
        let end = before[..end_char].iter().map(|c| c.len_utf16()).sum();
        let replacement: String = insertion.iter().collect();
        let edited = replace_text_range(&bytes, 0, 7, start, end, &replacement, false).unwrap();
        let expected: String = before[..start_char].iter().chain(&insertion).chain(&before[end_char..]).collect();
        proptest::prop_assert_eq!(shape_text(&edited, 0, 7).unwrap().text, expected);
        let old = OpcPackage::open(&bytes).unwrap();
        let new = OpcPackage::open(&edited).unwrap();
        for part in old.part_names() {
            if part != "ppt/slides/slide1.xml" { proptest::prop_assert_eq!(old.part(part), new.part(part)); }
        }
    }
}

#[test]
fn replaces_across_runs_and_keeps_styles_hyperlinks_and_other_parts() {
    let bytes = custom(
        r#"<a:p><a:pPr algn="ctr"/><a:r><a:rPr b="1"/><a:t>Hello </a:t></a:r><a:r><a:rPr i="1"><a:hlinkClick r:id="rId99"/></a:rPr><a:t>world</a:t></a:r><a:endParaRPr sz="2400"/></a:p>"#,
        "",
    );
    let edited = replace_text_range(&bytes, 0, 7, 3, 8, "新しい", false).unwrap();
    assert_eq!(shape_text(&edited, 0, 7).unwrap().text, "Hel新しいrld");
    let shape = text_shape(&edited);
    let body = shape.text.unwrap();
    assert_eq!(body.paragraphs.len(), 1);
    let nonempty: Vec<_> = body.paragraphs[0]
        .runs
        .iter()
        .filter(|r| !r.text.is_empty())
        .collect();
    assert!(nonempty[0].bold && nonempty[1].bold);
    assert!(nonempty[2].italic && nonempty[2].hyperlink.is_some());
    assert_eq!(
        body.paragraphs[0].align,
        Some(jade_pptx_reader::TextAlign::Center)
    );
    let before = OpcPackage::open(&bytes).unwrap();
    let after = OpcPackage::open(&edited).unwrap();
    for name in before.part_names() {
        if name != "ppt/slides/slide1.xml" {
            assert_eq!(before.part(name), after.part(name), "{name}");
        }
    }
}

#[test]
fn splits_and_joins_paragraphs_and_inserts_soft_breaks_without_losing_properties() {
    let bytes = custom(
        r#"<a:p><a:pPr marL="12700"><a:buChar char="•"/></a:pPr><a:r><a:rPr b="1"/><a:t>abc</a:t></a:r><a:endParaRPr sz="1800"/></a:p>"#,
        "",
    );
    let split = replace_text_range(&bytes, 0, 7, 1, 1, "X\nY\nZ", false).unwrap();
    assert_eq!(shape_text(&split, 0, 7).unwrap().text, "aX\nY\nZbc");
    let paragraphs = text_shape(&split).text.unwrap().paragraphs;
    assert_eq!(paragraphs.len(), 3);
    for p in paragraphs {
        assert_eq!(p.margin_left_emu, Some(12700));
        assert!(p.runs.iter().all(|r| r.bold));
        assert_eq!(p.end_size_pt, Some(18.0));
    }
    let joined = replace_text_range(&split, 0, 7, 2, 5, "-", false).unwrap();
    assert_eq!(shape_text(&joined, 0, 7).unwrap().text, "aX-Zbc");
    assert_eq!(text_shape(&joined).text.unwrap().paragraphs.len(), 1);
    let soft = replace_text_range(&bytes, 0, 7, 1, 1, "\n", true).unwrap();
    assert_eq!(shape_text(&soft, 0, 7).unwrap().text, "a\nbc");
    assert_eq!(text_shape(&soft).text.unwrap().paragraphs.len(), 1);
}

#[test]
fn validates_utf16_unicode_empty_paragraphs_and_undo_redo() {
    let bytes = custom(&p("A🦀日本"), "");
    assert!(replace_text_range(&bytes, 0, 7, 2, 3, "bad", false).is_err());
    assert!(replace_text_range(&bytes, 0, 7, 1, 2, "bad", false).is_err());
    assert!(replace_text_range(&bytes, 0, 7, 3, 1, "bad", false).is_err());
    assert!(replace_text_range(&bytes, 0, 7, 0, 99, "bad", false).is_err());
    let mut session = EditSession::open(&bytes).unwrap();
    session
        .replace_text_range(
            0,
            &TextRangeEdit {
                shape_id: 7,
                start: 1,
                end: 3,
                text: "🧑‍💻 & <text>".into(),
                soft_break: false,
            },
        )
        .unwrap();
    assert_eq!(
        shape_text(session.bytes(), 0, 7).unwrap().text,
        "A🧑‍💻 & <text>日本"
    );
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
    assert!(session.redo());
    let empty = custom("<a:p><a:endParaRPr sz=\"3200\" b=\"1\"/></a:p>", "");
    let added = replace_text_range(&empty, 0, 7, 0, 0, "text\nnext", false).unwrap();
    assert_eq!(shape_text(&added, 0, 7).unwrap().text, "text\nnext");
    let body = text_shape(&added).text.unwrap();
    assert!(body
        .paragraphs
        .iter()
        .flat_map(|p| &p.runs)
        .all(|r| r.bold && r.size_pt == Some(32.0)));
    let no_op = replace_text_range(&bytes, 0, 7, 1, 3, "🦀", false).unwrap();
    assert_eq!(no_op, bytes);
}

#[test]
fn moving_run_scopes_between_paragraphs_preserves_namespace_aliases() {
    let bytes = custom(
        r#"<a:p xmlns:z="http://schemas.openxmlformats.org/drawingml/2006/main"><z:pPr algn="l"/><z:r><z:t>first</z:t></z:r></a:p><a:p xmlns:z="urn:extension" xmlns:q="http://schemas.openxmlformats.org/drawingml/2006/main"><q:r><q:rPr i="1"/><q:t>second</q:t></q:r><q:endParaRPr sz="2400"/></a:p>"#,
        "",
    );
    let merged = replace_text_range(&bytes, 0, 7, 5, 6, "", false).unwrap();
    assert_eq!(shape_text(&merged, 0, 7).unwrap().text, "firstsecond");
    let xml = OpcPackage::open(&merged)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap();
    assert!(xml.contains("xmlns:q=\"http://schemas.openxmlformats.org/drawingml/2006/main\""));
    let body = text_shape(&merged).text.unwrap();
    assert!(body.paragraphs[0]
        .runs
        .iter()
        .any(|r| r.text == "second" && r.italic));
}

#[test]
fn respects_text_locks_and_leaves_fields_and_unknown_inline_content_untouched() {
    let locked = custom(&p("text"), "<a:spLocks noTextEdit=\"1\"/>");
    assert!(shape_text(&locked, 0, 7).unwrap().locked);
    assert!(replace_text_range(&locked, 0, 7, 0, 0, "bad", false).is_err());
    let address = editable_runs(&locked, 0).unwrap()[0].address;
    assert!(replace_run_text(&locked, 0, address, "bad").is_err());
    for body in [
        "<a:p><a:fld id=\"{x}\" type=\"slidenum\"><a:t>1</a:t></a:fld></a:p>",
        "<a:p><m:math xmlns:m=\"urn:math\"/></a:p>",
    ] {
        let bytes = custom(body, "");
        assert!(!shape_text(&bytes, 0, 7).unwrap().can_edit);
        assert!(replace_text_range(&bytes, 0, 7, 0, 0, "bad", false).is_err());
    }
}

#[test]
fn derives_grapheme_safe_splices_for_japanese_combining_marks_and_emoji() {
    for (before, after) in [
        ("👩", "👩‍💻"),
        ("か", "か\u{3099}"),
        ("A🦀B", "A日本🦀B"),
        ("same", "same"),
    ] {
        let bytes = custom(&p(before), "");
        let mut session = EditSession::open(&bytes).unwrap();
        let changed = session.set_shape_text(0, 7, after, false).unwrap();
        assert_eq!(shape_text(session.bytes(), 0, 7).unwrap().text, after);
        assert_eq!(changed, before != after);
    }
}

#[test]
fn text_apis_keep_grouped_shapes_supported() {
    let mut package = OpcPackage::open(&custom(&p("grouped"), "")).unwrap();
    let xml = package
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap()
        .replace("<p:sp>", "<p:grpSp><p:sp>")
        .replace("</p:sp>", "</p:sp></p:grpSp>");
    package.replace("ppt/slides/slide1.xml", xml.into_bytes());
    let bytes = package.write().unwrap();
    let address = editable_runs(&bytes, 0).unwrap()[0].address;
    let replaced = replace_run_text(&bytes, 0, address, "group text").unwrap();
    assert_eq!(shape_text(&replaced, 0, 7).unwrap().text, "group text");
}

#[test]
fn typing_after_a_formatted_break_inherits_its_run_properties() {
    let bytes = custom(
        "<a:p><a:r><a:t>a</a:t></a:r><a:br><a:rPr b=\"1\" sz=\"4800\"/></a:br></a:p>",
        "",
    );
    let edited = replace_text_range(&bytes, 0, 7, 2, 2, "text", false).unwrap();
    let body = text_shape(&edited).text.unwrap();
    let inserted = body.paragraphs[0]
        .runs
        .iter()
        .find(|run| run.text == "text")
        .unwrap();
    assert!(inserted.bold);
    assert_eq!(inserted.size_pt, Some(48.0));
    let soft = replace_text_range(&edited, 0, 7, 6, 6, "\n", true).unwrap();
    let xml = OpcPackage::open(&soft)
        .unwrap()
        .required_part_str("ppt/slides/slide1.xml")
        .unwrap();
    assert!(xml.contains("<jadeDraw:br") && xml.contains("sz=\"4800\""));
}
