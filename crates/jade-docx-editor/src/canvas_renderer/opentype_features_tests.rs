//! Word 2010 typography (`w14:ligatures`, `w14:numForm`, …): a run asking
//! for OpenType features is drawn shaped a stretch at a time, so a painter
//! that shapes can draw the font's ligatures; a plain run is not.

use super::recording_host::RecordingHost;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::{Op, RecordingPainter};

fn hello() -> WordDocument {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/hello.docx");
    let bytes = std::fs::read(path).expect("hello.docx");
    futures::executor::block_on(WordDocument::load(&bytes, &Options::default())).expect("load")
}

fn render(effects: Option<RunTextEffects>) -> Vec<Op> {
    let model = hello();
    let mut document = (**model.document.as_ref().expect("document")).clone();
    for element in document
        .body
        .iter_mut()
        .flat_map(|body| body.elements.iter_mut())
    {
        if let Some(paragraph) = element.as_paragraph_mut() {
            for run in paragraph.runs.iter_mut() {
                run.text = "office files".into();
                let mut properties = run.properties.clone().unwrap_or_default();
                properties.text_effects = effects.clone().map(Box::new);
                run.properties = Some(properties);
            }
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

#[test]
fn ligature_runs_are_drawn_shaped_with_their_features() {
    let ops = render(Some(RunTextEffects {
        ligatures: Some("standard".into()),
        ..Default::default()
    }));
    let shaped: Vec<&Op> = ops
        .iter()
        .filter(|op| matches!(op, Op::FillShapedText(..)))
        .collect();
    assert!(!shaped.is_empty(), "no shaped text in {ops:?}");
    let Op::FillShapedText(text, clusters, _) = shaped[0] else {
        unreachable!()
    };
    assert!(text.contains("ffi"), "{text}");
    // One cluster per character, at increasing x.
    assert_eq!(clusters.len(), text.chars().count());
    assert!(clusters.windows(2).all(|pair| pair[0].1 < pair[1].1));
    assert!(ops
        .iter()
        .any(|op| matches!(op, Op::SetFont(font) if font.starts_with("ot:liga"))));
}

#[test]
fn plain_runs_stay_on_the_per_character_path() {
    let ops = render(None);
    assert!(!ops.iter().any(|op| matches!(op, Op::FillShapedText(..))));
    assert!(!ops
        .iter()
        .any(|op| matches!(op, Op::SetFont(font) if font.contains("ot:"))));
}

#[test]
fn a_word_2007_compatibility_mode_document_draws_without_its_typography() {
    let ligatures = || {
        Some(RunTextEffects {
            ligatures: Some("standard".into()),
            ..Default::default()
        })
    };
    let shaped = |ops: &[Op]| ops.iter().any(|op| matches!(op, Op::FillShapedText(..)));
    let model = hello();
    let mut document = (**model.document.as_ref().expect("document")).clone();
    for element in document
        .body
        .iter_mut()
        .flat_map(|body| body.elements.iter_mut())
    {
        if let Some(paragraph) = element.as_paragraph_mut() {
            for run in paragraph.runs.iter_mut() {
                run.text = "office files".into();
                let mut properties = run.properties.clone().unwrap_or_default();
                properties.text_effects = ligatures().map(Box::new);
                run.properties = Some(properties);
            }
        }
    }
    let render_mode = |mode: u32| {
        let mut document = document.clone();
        document.east_asian_line_breaking.compatibility_mode = Some(mode);
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
    };
    assert!(
        shaped(&render_mode(14)),
        "Word 2010 layout shapes ligatures"
    );
    assert!(!shaped(&render_mode(12)), "Word 2007 layout does not");
    assert!(shaped(&render_mode(15)), "and the setting does not stick");
}
