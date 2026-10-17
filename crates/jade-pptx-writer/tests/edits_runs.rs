use jade_opc::OpcPackage;
use jade_pptx_writer::{editable_runs, replace_run_text, EditSession, RunAddress};

const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

fn custom_slide(body: &str) -> Vec<u8> {
    let mut package = OpcPackage::open(DECK).unwrap();
    package.replace("ppt/slides/slide1.xml", format!(
        "<s:sld xmlns:s=\"{P}\" xmlns:d=\"{A}\" xmlns:x=\"urn:extension\"><s:cSld><s:spTree>{body}</s:spTree></s:cSld></s:sld>"
    ).into_bytes());
    package.write().unwrap()
}

fn shape(text: &str) -> String {
    format!("<s:sp><s:nvSpPr><s:cNvPr id=\"7\" name=\"Title\"/></s:nvSpPr><s:spPr/><s:txBody><d:bodyPr/><d:lstStyle/><d:p><d:pPr algn=\"ctr\"/><d:r><d:rPr b=\"1\"><d:hlinkClick xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:id=\"rId99\"/></d:rPr>{text}</d:r><d:br/><d:fld id=\"{{x}}\" type=\"slidenum\"><d:t>1</d:t></d:fld><d:r><d:t>second</d:t></d:r><d:endParaRPr sz=\"2400\"/></d:p></s:txBody><s:extLst><x:payload keep=\"yes\"/></s:extLst></s:sp>")
}

#[test]
fn changes_only_the_selected_payload_and_preserves_every_other_part() {
    let original = custom_slide(&format!(
        "<s:grpSp>{}</s:grpSp>",
        shape("<d:t xml:space=\"preserve\"> old &amp; text </d:t>")
    ));
    let runs = editable_runs(&original, 0).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].text, " old & text ");
    let replacement = " 新しい & <text> 🦀 ";
    let output = replace_run_text(&original, 0, runs[0].address, replacement).unwrap();
    assert_eq!(editable_runs(&output, 0).unwrap()[0].text, replacement);
    let before = OpcPackage::open(&original).unwrap();
    let after = OpcPackage::open(&output).unwrap();
    assert_eq!(before.part_names(), after.part_names());
    for name in before.part_names() {
        if name == "ppt/slides/slide1.xml" {
            let expected = before
                .required_part_str(name)
                .unwrap()
                .replace(" old &amp; text ", " 新しい &amp; &lt;text&gt; 🦀 ");
            assert_eq!(after.required_part_str(name).unwrap(), expected);
        } else {
            assert_eq!(before.part(name), after.part(name), "{name}");
        }
    }
    jade_pptx_reader::read_deck(&output).unwrap();
}

#[test]
fn expands_empty_text_and_keeps_cdata_and_entities_semantically_correct() {
    for payload in [
        "<d:t xml:space=\"preserve\"/>",
        "<d:t><![CDATA[<start>]]>&#x20;&amp;</d:t>",
    ] {
        let bytes = custom_slide(&shape(payload));
        let address = editable_runs(&bytes, 0).unwrap()[0].address;
        let saved = replace_run_text(&bytes, 0, address, "hello").unwrap();
        assert_eq!(editable_runs(&saved, 0).unwrap()[0].text, "hello");
        if payload.contains("xml:space") {
            assert!(OpcPackage::open(&saved)
                .unwrap()
                .required_part_str("ppt/slides/slide1.xml")
                .unwrap()
                .contains("<d:t xml:space=\"preserve\">hello</d:t>"));
        }
        let cleared = replace_run_text(&saved, 0, address, "").unwrap();
        assert_eq!(editable_runs(&cleared, 0).unwrap()[0].text, "");
    }
}

#[test]
fn ignores_foreign_namespace_nodes_and_only_edits_alternate_content_fallback() {
    let bytes = custom_slide(&format!("<x:sp>{}</x:sp><s:extLst>{}</s:extLst><mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice Requires=\"x\">{}</mc:Choice><mc:Fallback>{}</mc:Fallback></mc:AlternateContent>", shape("<d:t>foreign</d:t>"), shape("<d:t>extension</d:t>"), shape("<d:t>choice</d:t>"), shape("<d:t>fallback</d:t>")));
    let runs = editable_runs(&bytes, 0).unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].text, "fallback");
}

#[test]
fn no_op_invalid_and_ambiguous_edits_do_not_change_the_document() {
    let mut session = EditSession::open(DECK).unwrap();
    let run = editable_runs(DECK, 0).unwrap().remove(0);
    assert!(!session.set_run_text(0, run.address, &run.text).unwrap());
    assert!(!session.is_dirty());
    assert!(!session.can_undo());
    for text in ["bad\0", "bad\u{000B}", "new\nparagraph"] {
        assert!(session.set_run_text(0, run.address, text).is_err());
        assert_eq!(session.bytes(), DECK);
    }
    assert!(session.set_run_text(99, run.address, "bad").is_err());
    assert!(session
        .set_run_text(
            0,
            RunAddress {
                shape_id: 9999,
                ..run.address
            },
            "bad"
        )
        .is_err());
    let duplicate = custom_slide(&(shape("<d:t>one</d:t>") + &shape("<d:t>two</d:t>")));
    let address = editable_runs(&duplicate, 0).unwrap()[0].address;
    assert!(replace_run_text(&duplicate, 0, address, "ambiguous").is_err());
}

#[test]
fn undo_redo_saved_baseline_and_branching_survive_export_and_reopen() {
    let run = editable_runs(DECK, 0).unwrap().remove(0);
    let mut session = EditSession::open(DECK).unwrap();
    assert!(!session.undo());
    session.set_run_text(0, run.address, "first").unwrap();
    assert!(session.is_dirty());
    session.mark_saved();
    assert!(!session.is_dirty());
    session.set_run_text(0, run.address, "second").unwrap();
    assert!(session.undo());
    assert!(!session.is_dirty());
    assert!(session.undo());
    assert_eq!(session.bytes(), DECK);
    assert!(session.is_dirty());
    assert!(session.redo());
    assert!(!session.is_dirty());
    assert!(session.redo());
    assert_eq!(editable_runs(session.bytes(), 0).unwrap()[0].text, "second");
    assert!(session.undo());
    session.set_run_text(0, run.address, "branch").unwrap();
    assert!(!session.can_redo());
    let reopened = EditSession::open(session.bytes()).unwrap();
    assert!(!reopened.is_dirty());
    assert_eq!(
        editable_runs(reopened.bytes(), 0).unwrap()[0].text,
        "branch"
    );
}
