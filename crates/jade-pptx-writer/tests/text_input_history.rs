use jade_opc::OpcPackage;
use jade_pptx_reader::ShapeItem;
use jade_pptx_writer::{
    shape_text, EditSession, TextFormat, TextFormatEdit, TextInputEdit, TypingKind,
};
const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn custom(body: &str, locks: &str) -> Vec<u8> {
    let mut package = OpcPackage::open(DECK).unwrap();
    package.replace("ppt/slides/slide1.xml", format!(r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="7" name="Text box"/><p:cNvSpPr txBox="1">{locks}</p:cNvSpPr><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="9144000" cy="4572000"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/>{body}</p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#).into_bytes());
    package.write().unwrap()
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

fn input(
    session: &EditSession,
    start: usize,
    end: usize,
    text: &str,
    history: Option<TypingKind>,
) -> TextInputEdit {
    TextInputEdit {
        shape_id: 7,
        start,
        end,
        text: text.into(),
        expected_text: shape_text(session.bytes(), 0, 7).unwrap().text,
        soft_break: false,
        history,
        format: TextFormat::default(),
    }
}
#[test]
fn a_long_contiguous_typing_burst_uses_one_snapshot_and_undo_redo_preserve_exact_bytes() {
    let original = custom("<a:p><a:r><a:t>start</a:t></a:r></a:p>", "");
    let mut s = EditSession::open(&original).unwrap();
    for i in 0..120 {
        let result = s
            .apply_text_input(0, &input(&s, 5 + i, 5 + i, "x", Some(TypingKind::Insert)))
            .unwrap();
        assert_eq!(result.coalesced, i > 0);
    }
    assert_eq!(s.history_depth(), (1, 0));
    let edited = s.bytes().to_vec();
    assert_eq!(shape_text(&edited, 0, 7).unwrap().text.len(), 125);
    assert!(s.undo());
    assert_eq!(s.bytes(), original);
    assert_eq!(s.history_depth(), (0, 1));
    assert!(!s.is_dirty());
    assert!(s.redo());
    assert_eq!(s.bytes(), edited);
    assert_eq!(s.history_depth(), (1, 0));
}
#[test]
fn backward_and_forward_deletion_form_separate_compatible_groups() {
    let bytes = custom("<a:p><a:r><a:t>abcdef</a:t></a:r></a:p>", "");
    let mut s = EditSession::open(&bytes).unwrap();
    s.apply_text_input(0, &input(&s, 5, 6, "", Some(TypingKind::Backspace)))
        .unwrap();
    assert!(
        s.apply_text_input(0, &input(&s, 4, 5, "", Some(TypingKind::Backspace)))
            .unwrap()
            .coalesced
    );
    s.apply_text_input(0, &input(&s, 0, 1, "", Some(TypingKind::Delete)))
        .unwrap();
    assert!(
        s.apply_text_input(0, &input(&s, 0, 1, "", Some(TypingKind::Delete)))
            .unwrap()
            .coalesced
    );
    assert_eq!(shape_text(s.bytes(), 0, 7).unwrap().text, "cd");
    assert_eq!(s.history_depth(), (2, 0));
    s.undo();
    assert_eq!(shape_text(s.bytes(), 0, 7).unwrap().text, "abcd");
    s.undo();
    assert_eq!(s.bytes(), bytes);
}
#[test]
fn caret_boundaries_atomic_paste_and_newlines_break_groups() {
    let bytes = custom("<a:p><a:r><a:t>ab</a:t></a:r></a:p>", "");
    let mut s = EditSession::open(&bytes).unwrap();
    s.apply_text_input(0, &input(&s, 2, 2, "c", Some(TypingKind::Insert)))
        .unwrap();
    s.end_text_history();
    assert!(
        !s.apply_text_input(0, &input(&s, 3, 3, "d", Some(TypingKind::Insert)))
            .unwrap()
            .coalesced
    );
    assert!(
        !s.apply_text_input(0, &input(&s, 4, 4, "PASTE", None))
            .unwrap()
            .coalesced
    );
    assert!(
        !s.apply_text_input(0, &input(&s, 9, 9, "e", Some(TypingKind::Insert)))
            .unwrap()
            .coalesced
    );
    s.apply_text_input(0, &input(&s, 10, 10, "\n", Some(TypingKind::Insert)))
        .unwrap();
    assert!(
        !s.apply_text_input(0, &input(&s, 11, 11, "f", Some(TypingKind::Insert)))
            .unwrap()
            .coalesced
    );
    assert_eq!(s.history_depth(), (6, 0));
}
#[test]
fn save_undo_redo_and_format_changes_do_not_merge_across_boundaries() {
    let bytes = custom("<a:p><a:r><a:t>a</a:t></a:r></a:p>", "");
    let mut s = EditSession::open(&bytes).unwrap();
    s.apply_text_input(0, &input(&s, 1, 1, "b", Some(TypingKind::Insert)))
        .unwrap();
    s.mark_saved();
    let saved = s.bytes().to_vec();
    assert!(
        !s.apply_text_input(0, &input(&s, 2, 2, "c", Some(TypingKind::Insert)))
            .unwrap()
            .coalesced
    );
    s.undo();
    assert_eq!(s.bytes(), saved);
    assert!(!s.is_dirty());
    s.redo();
    assert!(
        !s.apply_text_input(0, &input(&s, 3, 3, "d", Some(TypingKind::Insert)))
            .unwrap()
            .coalesced
    );
    s.format_text_range(
        0,
        &TextFormatEdit {
            shape_id: 7,
            start: 0,
            end: 1,
            format: TextFormat {
                bold: Some(true),
                ..Default::default()
            },
        },
    )
    .unwrap();
    assert!(
        !s.apply_text_input(0, &input(&s, 4, 4, "e", Some(TypingKind::Insert)))
            .unwrap()
            .coalesced
    );
}
#[test]
fn repeated_text_insertion_uses_the_actual_caret_style_and_formats_only_the_inserted_range() {
    let bytes=custom("<a:p><a:r><a:rPr b=\"1\"/><a:t>aa</a:t></a:r><a:r><a:rPr i=\"1\"/><a:t>aa</a:t></a:r></a:p>","");
    let mut s = EditSession::open(&bytes).unwrap();
    let mut edit = input(&s, 1, 1, "a", Some(TypingKind::Insert));
    edit.format = TextFormat {
        underline: Some(true),
        ..Default::default()
    };
    s.apply_text_input(0, &edit).unwrap();
    let runs = &paragraphs(s.bytes())[0].runs;
    assert_eq!(
        runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(),
        ["a", "a", "a", "aa"]
    );
    assert!(runs[1].bold && runs[1].underline && !runs[1].italic);
    assert!(!runs[0].underline && !runs[2].underline);
    assert!(runs[3].italic);
    assert!(xml(s.bytes()).contains("i=\"1\""));
    s.undo();
    assert_eq!(s.bytes(), bytes);
}
#[test]
fn stale_invalid_and_locked_input_preserve_the_document_and_current_history_group() {
    let bytes = custom("<a:p><a:r><a:t>🦀</a:t></a:r></a:p>", "");
    let mut s = EditSession::open(&bytes).unwrap();
    s.apply_text_input(0, &input(&s, 2, 2, "a", Some(TypingKind::Insert)))
        .unwrap();
    let snapshot = s.bytes().to_vec();
    let mut stale = input(&s, 3, 3, "b", Some(TypingKind::Insert));
    stale.expected_text = "old".into();
    assert!(s.apply_text_input(0, &stale).is_err());
    assert!(s
        .apply_text_input(0, &input(&s, 1, 1, "x", Some(TypingKind::Insert)))
        .is_err());
    assert_eq!(s.bytes(), snapshot);
    assert_eq!(s.history_depth(), (1, 0));
    assert!(
        s.apply_text_input(0, &input(&s, 3, 3, "b", Some(TypingKind::Insert)))
            .unwrap()
            .coalesced
    );
    let locked = custom(
        "<a:p><a:r><a:t>x</a:t></a:r></a:p>",
        "<a:spLocks noTextEdit=\"1\"/>",
    );
    let mut locked = EditSession::open(&locked).unwrap();
    assert!(locked
        .apply_text_input(0, &input(&locked, 1, 1, "y", Some(TypingKind::Insert)))
        .is_err());
    assert!(!locked.can_undo());
}

#[test]
fn notes_creation_and_typing_coalesce_without_merging_with_slide_text() {
    use jade_pptx_writer::{notes_text, NotesInputEdit};
    let mut s = EditSession::open(DECK).unwrap();
    let original = s.bytes().to_vec();
    let mut value = String::new();
    for chunk in ["発表", "者", "ノート"] {
        let at = value.encode_utf16().count();
        let result = s
            .apply_notes_input(
                0,
                &NotesInputEdit {
                    start: at,
                    end: at,
                    text: chunk.into(),
                    expected_text: value.clone(),
                    soft_break: false,
                    history: Some(TypingKind::Insert),
                },
            )
            .unwrap();
        assert_eq!(result.coalesced, !value.is_empty());
        value.push_str(chunk);
    }
    assert_eq!(notes_text(s.bytes(), 0).unwrap().text, value);
    assert_eq!(s.history_depth(), (1, 0));
    let notes = s.bytes().to_vec();
    s.undo();
    assert_eq!(s.bytes(), original);
    s.redo();
    assert_eq!(s.bytes(), notes);
    s.end_text_history();
    let edit = TextInputEdit {
        shape_id: 2,
        start: 11,
        end: 11,
        text: "X".into(),
        expected_text: "Jade Slides".into(),
        soft_break: false,
        history: Some(TypingKind::Insert),
        format: TextFormat::default(),
    };
    assert!(!s.apply_text_input(0, &edit).unwrap().coalesced);
    assert_eq!(s.history_depth(), (2, 0));
    let bad = NotesInputEdit {
        start: 0,
        end: 0,
        text: "bad".into(),
        expected_text: "stale".into(),
        soft_break: false,
        history: Some(TypingKind::Insert),
    };
    let before = s.bytes().to_vec();
    assert!(s.apply_notes_input(0, &bad).is_err());
    assert_eq!(s.bytes(), before);
}

#[test]
fn notes_range_input_preserves_the_style_at_a_repeated_text_insertion() {
    use jade_pptx_writer::{notes_text, set_slide_notes, NotesInputEdit};
    let generated = set_slide_notes(DECK, 0, "aaaa").unwrap();
    let mut p = OpcPackage::open(&generated).unwrap();
    let part = p
        .part_names()
        .iter()
        .find(|name| {
            name.starts_with("ppt/notesSlides/")
                && name.ends_with(".xml")
                && p.part_str(name)
                    .unwrap()
                    .is_some_and(|s| s.contains("aaaa"))
        })
        .unwrap()
        .to_owned();
    let slide=custom("<a:p><a:r><a:rPr b=\"1\" i=\"0\"/><a:t>aa</a:t></a:r><a:r><a:rPr b=\"0\" i=\"1\"/><a:t>aa</a:t></a:r></a:p>","");
    let note = xml(&slide)
        .replace("p:sld", "p:notes")
        .replace("<p:nvPr/>", "<p:nvPr><p:ph type=\"body\"/></p:nvPr>");
    p.replace(&part, note.into_bytes());
    let bytes = p.write().unwrap();
    let mut s = EditSession::open(&bytes).unwrap();
    s.apply_notes_input(
        0,
        &NotesInputEdit {
            start: 1,
            end: 1,
            text: "a".into(),
            expected_text: "aaaa".into(),
            soft_break: false,
            history: Some(TypingKind::Insert),
        },
    )
    .unwrap();
    let note = OpcPackage::open(s.bytes())
        .unwrap()
        .required_part_str(&part)
        .unwrap();
    let content = jade_pptx_reader::parse_slide_xml(&note.replace("p:notes", "p:sld")).unwrap();
    let ShapeItem::Shape(shape) = &content.shapes[0] else {
        panic!("shape")
    };
    let runs = &shape.text.as_ref().unwrap().paragraphs[0].runs;
    assert!(runs[1].bold && !runs[1].italic);
    assert!(runs.last().unwrap().italic);
    assert_eq!(notes_text(s.bytes(), 0).unwrap().text, "aaaaa");
    s.undo();
    assert_eq!(s.bytes(), bytes);
}
