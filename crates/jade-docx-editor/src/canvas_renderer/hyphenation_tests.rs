//! Word's automatic hyphenation (`w:autoHyphenation`): lines end in a hyphen
//! the text does not have, unless the paragraph suppresses it.

use super::recording_host::RecordingHost;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::{Op, RecordingPainter};

const TEXT: &str = "Automatic hyphenation divides considerable international \
    responsibilities between representatives of organizations, documentation \
    departments and administrative committees, characteristically \
    accommodating extraordinary circumstances whenever practicable.";

fn render(auto: bool, suppress: bool) -> Vec<Op> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    let bytes = std::fs::read(path).expect("hello.docx");
    let model =
        futures::executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load");
    let mut document = (**model.document.as_ref().expect("document")).clone();
    document.hyphenation.auto = auto;
    let mut first = true;
    for element in document
        .body
        .iter_mut()
        .flat_map(|body| body.elements.iter_mut())
    {
        if let Some(paragraph) = element.as_paragraph_mut() {
            for run in paragraph.runs.iter_mut() {
                run.text = if first { TEXT.into() } else { String::new() };
                first = false;
            }
            paragraph
                .properties
                .get_or_insert_with(Default::default)
                .suppress_auto_hyphens = Some(suppress);
        }
    }
    let painter = RecordingPainter::new();
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: painter.clone(),
        }),
        false,
    )
    .expect("recording renderer");
    renderer
        .render(&Rc::new(document), &Options::default())
        .expect("render");
    painter.ops()
}

fn hyphens(ops: &[Op]) -> usize {
    ops.iter()
        .filter(|op| matches!(op, Op::FillText(text, _, _) if text == "-"))
        .count()
}

#[test]
fn automatic_hyphenation_draws_hyphens_at_line_ends() {
    assert!(!TEXT.contains('-'));
    assert_eq!(hyphens(&render(false, false)), 0);
    assert!(hyphens(&render(true, false)) > 0);
}

#[test]
fn a_paragraph_can_suppress_automatic_hyphenation() {
    assert_eq!(hyphens(&render(true, true)), 0);
}
