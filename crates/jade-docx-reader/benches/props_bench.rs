//! Criterion wall-clock benchmark for the run-style property hot paths
//! (`RunProperties::clone` per render run, and `merge_run_properties_override`
//! per style-resolution step). Compiles identically on the `String` baseline
//! and the `Rc<str>` branch — field values use `.into()`, which works for both
//! `String` and `Rc<str>` — so the same bench measures both checkouts.
//!
//! Run: `cargo bench -p jade-docx-reader`

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use jade_docx_reader::styles::{
    merge_paragraph_properties_override, merge_run_properties_override,
};
use jade_docx_reader::{DocxXmlDocument, ParagraphProperties, RunFonts, RunProperties, XmlNode};

/// A representative directly-formatted run: explicit fonts (latin + east-asia),
/// a color, a font_family and a character-style reference.
fn representative() -> RunProperties {
    RunProperties {
        bold: Some(true),
        font_size: Some(10.5),
        style: Some("BodyText".into()),
        font_family: Some("Yu Mincho".into()),
        color: Some("#1A1A1A".into()),
        fonts: Some(RunFonts {
            ascii: Some("Yu Mincho".into()),
            h_ansi: Some("Yu Mincho".into()),
            east_asia: Some("游明朝".into()),
            ..RunFonts::default()
        }),
        ..RunProperties::default()
    }
}

/// A representative paragraph: a style reference, alignment and text-alignment.
fn representative_paragraph() -> ParagraphProperties {
    ParagraphProperties {
        style: Some("Heading2".into()),
        alignment: Some("center".into()),
        text_alignment: Some("baseline".into()),
        ..ParagraphProperties::default()
    }
}

fn bench_props(c: &mut Criterion) {
    let source = representative();

    c.bench_function("run_properties_clone", |b| {
        b.iter(|| black_box(black_box(&source).clone()));
    });

    c.bench_function("merge_run_properties_override", |b| {
        b.iter(|| {
            let mut target = RunProperties::default();
            merge_run_properties_override(&mut target, black_box(&source));
            black_box(&target);
        });
    });

    let psource = representative_paragraph();

    c.bench_function("paragraph_properties_clone", |b| {
        b.iter(|| black_box(black_box(&psource).clone()));
    });

    c.bench_function("merge_paragraph_properties_override", |b| {
        b.iter(|| {
            let mut target = ParagraphProperties::default();
            merge_paragraph_properties_override(&mut target, black_box(&psource));
            black_box(&target);
        });
    });
}

fn large_docx_xml(paragraphs: usize, runs_per_paragraph: usize) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
    );
    for paragraph_index in 0..paragraphs {
        xml.push_str("<w:p>");
        for run_index in 0..runs_per_paragraph {
            xml.push_str("<w:r><w:t>");
            xml.push_str(&format!("{paragraph_index}-{run_index}"));
            xml.push_str("</w:t></w:r>");
        }
        xml.push_str("</w:p>");
    }
    xml.push_str("</w:body></w:document>");
    xml
}

fn body_id(doc: &DocxXmlDocument) -> jade_docx_reader::XmlNodeId {
    doc.root()
        .children
        .iter()
        .find_map(|node| match node {
            XmlNode::Element(element) if element.name == "w:body" => Some(element.id),
            _ => None,
        })
        .expect("body id")
}

fn first_paragraph_id(doc: &DocxXmlDocument) -> jade_docx_reader::XmlNodeId {
    let body = body_id(doc);
    doc.element(body)
        .expect("body")
        .children
        .iter()
        .find_map(|node| match node {
            XmlNode::Element(element) if element.name == "w:p" => Some(element.id),
            _ => None,
        })
        .expect("paragraph id")
}

fn first_run_id(doc: &DocxXmlDocument) -> jade_docx_reader::XmlNodeId {
    let paragraph = first_paragraph_id(doc);
    doc.element(paragraph)
        .expect("paragraph")
        .children
        .iter()
        .find_map(|node| match node {
            XmlNode::Element(element) if element.name == "w:r" => Some(element.id),
            _ => None,
        })
        .expect("run id")
}

fn bench_docx_xml_path_updates(c: &mut Criterion) {
    let xml = large_docx_xml(512, 16);
    let source = DocxXmlDocument::parse(&xml).expect("parse large xml");

    c.bench_function("docx_xml_rebuild_paragraph_index", |b| {
        let mut doc = source.clone();
        let paragraph = first_paragraph_id(&doc);
        b.iter(|| {
            doc.rebuild_index_for_subtree(black_box(paragraph));
            black_box(&doc);
        });
    });

    c.bench_function("docx_xml_remove_run_path_update", |b| {
        b.iter_batched(
            || source.clone(),
            |mut doc| {
                let run = first_run_id(&doc);
                let path = doc.path(run).expect("run path");
                black_box(doc.remove_node(&path).expect("remove run"));
                black_box(doc);
            },
            BatchSize::SmallInput,
        );
    });

    c.bench_function("docx_xml_insert_run_path_update", |b| {
        b.iter_batched(
            || source.clone(),
            |mut doc| {
                let paragraph = first_paragraph_id(&doc);
                let text =
                    doc.create_element("w:t", Vec::new(), vec![XmlNode::Text("Inserted".into())]);
                let run = doc.create_element("w:r", Vec::new(), vec![XmlNode::Element(text)]);
                black_box(
                    doc.insert_node(paragraph, 0, XmlNode::Element(run))
                        .expect("insert run"),
                );
                black_box(doc);
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_word_namespaces(c: &mut Criterion) {
    let canonical = large_docx_xml(4096, 16);
    let aliased = canonical
        .replace("w:", "ns0:")
        .replace("xmlns:w=", "xmlns:ns0=");
    for (name, xml) in [("canonical", &canonical), ("aliased", &aliased)] {
        c.bench_function(&format!("word_namespaces_4096_paragraphs_{name}"), |b| {
            b.iter(|| {
                black_box(jade_docx_reader::word_namespaces::normalize(black_box(xml)).unwrap())
            });
        });
    }
}

criterion_group!(
    benches,
    bench_props,
    bench_docx_xml_path_updates,
    bench_word_namespaces
);
criterion_main!(benches);
