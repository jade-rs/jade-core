//! Criterion wall-clock benchmark for the run-style property hot paths
//! (`RunProperties::clone` per render run, and `merge_run_properties_override`
//! per style-resolution step). Compiles identically on the `String` baseline
//! and the `Rc<str>` branch — field values use `.into()`, which works for both
//! `String` and `Rc<str>` — so the same bench measures both checkouts.
//!
//! Run: `cargo bench -p jade-docx-reader`

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use jade_docx_reader::numbering::NumberingPart;
use jade_docx_reader::styles::{
    merge_paragraph_properties_override, merge_run_properties_override, StylesPart,
};
use jade_docx_reader::{
    DocumentParser, DocxXmlDocument, ParagraphProperties, RunFonts, RunProperties, TocOptions,
    XmlNode,
};
use std::io::Read;

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

    fn collect_ids(
        element: &jade_docx_reader::XmlElement,
        ids: &mut Vec<jade_docx_reader::XmlNodeId>,
    ) {
        ids.push(element.id);
        for child in &element.children {
            if let XmlNode::Element(child) = child {
                collect_ids(child, ids);
            }
        }
    }
    let mut ids = Vec::new();
    collect_ids(source.root(), &mut ids);
    c.bench_function("docx_xml_element_lookup", |b| {
        b.iter(|| {
            for id in &ids {
                black_box(source.element(black_box(*id)));
            }
        });
    });
    c.bench_function("docx_xml_parent_lookup", |b| {
        b.iter(|| {
            for id in &ids {
                black_box(source.parent_id(black_box(*id)));
            }
        });
    });
    // Field mapping repeatedly follows ancestors from a run to its paragraph.
    let run = first_run_id(&source);
    c.bench_function("docx_xml_parent_ancestors", |b| {
        b.iter(|| {
            let mut current = black_box(run);
            while let Some(parent) = source.parent_id(current) {
                current = black_box(parent);
            }
        });
    });

    c.bench_function("docx_xml_rebuild_paragraph_index", |b| {
        let mut doc = source.clone();
        let paragraph = first_paragraph_id(&doc);
        b.iter(|| {
            doc.rebuild_index_for_subtree(black_box(paragraph));
            black_box(&doc);
        });
    });

    c.bench_function("docx_xml_rebuild_body_index", |b| {
        let mut doc = source.clone();
        let body = body_id(&doc);
        b.iter(|| {
            doc.rebuild_index_for_subtree(black_box(body));
            black_box(&doc);
        });
    });

    c.bench_function("docx_xml_rebuild_full_index", |b| {
        let mut doc = source.clone();
        b.iter(|| {
            doc.rebuild_index();
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

// Isolate repeated heading-style recognition from XML parsing. Normal paragraphs
// exercise rejection; heading paragraphs also include TOC entry allocation.
fn bench_outline(c: &mut Criterion) {
    let mut group = c.benchmark_group("generate_outline");
    for style in ["Normal", "Heading1"] {
        let mut xml = String::from(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
        );
        for _ in 0..1000 {
            xml.push_str(&format!(
                r#"<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:r><w:t>Paragraph text</w:t></w:r></w:p>"#,
            ));
        }
        xml.push_str("</w:body></w:document>");
        let document = DocumentParser::new()
            .parse_document_xml(&xml)
            .expect("parse outline fixture");
        let options = TocOptions::default();
        assert_eq!(
            document.generate_outline(&options).len(),
            if style == "Heading1" { 1000 } else { 0 }
        );
        group.bench_function(format!("paragraphs=1000/style={style}"), |b| {
            b.iter(|| black_box(document.generate_outline(black_box(&options))));
        });
    }
    group.finish();
}

fn bench_run_font_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_run_fonts");
    for (name, ending) in [("empty", "/>"), ("start_end", "></w:rFonts>")] {
        let paragraph = format!(
            r#"<w:p><w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:eastAsia="Japanese"{ending}</w:rPr></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="Japanese" w:hint="eastAsia"{ending}</w:rPr><w:t>Formatted text</w:t></w:r></w:p>"#,
        );
        let xml = format!(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{}</w:body></w:document>"#,
            paragraph.repeat(1000),
        );
        group.bench_function(name, |b| {
            b.iter(|| {
                black_box(
                    DocumentParser::new()
                        .parse_document_xml(black_box(&xml))
                        .expect("parse formatted document"),
                )
            });
        });
        group.bench_function(format!("{name}_with_raw_tree"), |b| {
            b.iter(|| {
                let (document, raw) = DocumentParser::new()
                    .parse_document_xml_with_raw_tree(black_box(&xml))
                    .expect("parse formatted document");
                black_box((document, raw.expect("retain XML tree")))
            });
        });
    }
    group.finish();
}

fn bench_paragraph_style_resolution(c: &mut Criterion) {
    let mut group = c.benchmark_group("paragraph_style_resolution");
    for depth in [1, 4, 8, 16, 64] {
        let mut xml = String::from(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
        );
        for index in 0..depth {
            xml.push_str(&format!(
                r#"<w:style w:type="paragraph" w:styleId="Style{index}">"#,
            ));
            if index > 0 {
                xml.push_str(&format!(r#"<w:basedOn w:val="Style{}"/>"#, index - 1));
            }
            xml.push_str("<w:pPr>");
            if index == 0 {
                xml.push_str(r#"<w:jc w:val="left"/><w:spacing w:after="120"/>"#);
            }
            xml.push_str(&format!(
                r#"<w:ind w:left="{}"/></w:pPr></w:style>"#,
                index * 20,
            ));
        }
        xml.push_str("</w:styles>");
        let styles = StylesPart::from_xml(&xml).expect("parse style chain");
        let style_id = format!("Style{}", depth - 1);
        let resolved = styles
            .resolve_paragraph_properties(&style_id)
            .expect("resolve style");
        assert_eq!(resolved.alignment.as_deref(), Some("left"));
        group.bench_function(format!("depth={depth}"), |b| {
            b.iter(|| black_box(styles.resolve_paragraph_properties(black_box(&style_id))));
        });
    }
    group.finish();
}

fn bench_numbering_parse(c: &mut Criterion) {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(include_bytes!(
        "../../../contract.docx"
    )))
    .expect("open contract");
    let mut contract_xml = String::new();
    archive
        .by_name("word/numbering.xml")
        .expect("numbering part")
        .read_to_string(&mut contract_xml)
        .expect("read numbering XML");
    let mut fixtures = vec![("contract".to_owned(), contract_xml)];
    for definitions in [1, 100] {
        let mut xml = String::from(
            r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
        );
        for id in 0..definitions {
            xml.push_str(&format!(r#"<w:abstractNum w:abstractNumId="{id}">"#));
            for level in 0..9 {
                xml.push_str(&format!(
                    r#"<w:lvl w:ilvl="{level}"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial" w:eastAsia="游明朝"/><w:sz w:val="22"/></w:rPr></w:lvl>"#,
                ));
            }
            xml.push_str(&format!(
                r#"</w:abstractNum><w:num w:numId="{id}"><w:abstractNumId w:val="{id}"/></w:num>"#,
            ));
        }
        xml.push_str("</w:numbering>");
        fixtures.push((format!("definitions={definitions}"), xml));
    }
    let mut group = c.benchmark_group("numbering_parse");
    for (name, xml) in fixtures {
        assert!(!NumberingPart::from_xml(&xml)
            .unwrap()
            .abstract_nums
            .is_empty());
        group.bench_function(name, |b| {
            b.iter(|| {
                black_box(NumberingPart::from_xml(black_box(&xml)).expect("parse numbering"))
            });
        });
    }
    group.finish();
}

fn bench_docx_xml_parse(c: &mut Criterion) {
    let mut fixtures = Vec::new();
    for (name, bytes) in [
        (
            "contract",
            include_bytes!("../../../contract.docx").as_slice(),
        ),
        (
            "yokohama",
            include_bytes!("../../../public/fixtures/yokohama.docx").as_slice(),
        ),
    ] {
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("open document fixture");
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .expect("document part")
            .read_to_string(&mut xml)
            .expect("read document XML");
        fixtures.push((name.to_owned(), xml));
    }
    for count in [0, 1, 4, 5, 6, 8, 16] {
        let mut xml = String::from("<root>");
        for _ in 0..1000 {
            xml.push_str("<element");
            for index in 0..count {
                xml.push_str(&format!(" attr{index}=\"value\""));
            }
            xml.push_str("/>");
        }
        xml.push_str("</root>");
        fixtures.push((format!("attributes={count}"), xml));
    }
    for (name, text) in [
        ("plain", "Ordinary text"),
        ("named", "A &amp; B &lt; C &gt; D &quot; E &apos; F"),
        (
            "numeric",
            "A &#38; B &#60; C &#62; D &#34; E &#39; F &#128512;",
        ),
        ("mixed", "A &amp; B &#x1F600; C &lt; D &#10; E"),
    ] {
        let mut xml = String::from("<w:document><w:body>");
        for _ in 0..512 {
            xml.push_str("<w:p>");
            for _ in 0..8 {
                xml.push_str("<w:r><w:t>");
                xml.push_str(text);
                xml.push_str("</w:t></w:r>");
            }
            xml.push_str("</w:p>");
        }
        xml.push_str("</w:body></w:document>");
        fixtures.push((format!("entities={name}"), xml));
    }
    let mut group = c.benchmark_group("docx_xml_parse");
    for (name, xml) in fixtures {
        DocxXmlDocument::parse(&xml).expect("validate XML fixture");
        group.bench_function(name.as_str(), |b| {
            b.iter(|| black_box(DocxXmlDocument::parse(black_box(&xml)).expect("parse XML")));
        });
        if name == "contract" || name == "yokohama" {
            {
                let (_, raw) = DocumentParser::new()
                    .parse_document_xml_with_raw_tree(&xml)
                    .expect("parse document");
                let raw = raw.expect("build raw XML tree");
                group.bench_function(format!("{name}_write_from_model"), |b| {
                    b.iter(|| black_box(black_box(&raw).to_string().expect("write XML")));
                });
            }
            group.bench_function(format!("{name}_with_model_and_write"), |b| {
                b.iter(|| {
                    let (model, raw) = DocumentParser::new()
                        .parse_document_xml_with_raw_tree(black_box(&xml))
                        .expect("parse document");
                    black_box((
                        model,
                        raw.expect("build raw XML tree")
                            .to_string()
                            .expect("write XML"),
                    ))
                });
            });
            group.bench_function(format!("{name}_with_model"), |b| {
                b.iter(|| {
                    black_box(
                        DocumentParser::new()
                            .parse_document_xml_with_raw_tree(black_box(&xml))
                            .expect("parse document"),
                    )
                });
            });
        }
    }
    group.finish();
}

fn bench_on_off_parse(c: &mut Criterion) {
    let mut fixtures = Vec::new();
    for kind in ["run", "paragraph"] {
        let mut xml = String::from(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
        );
        for index in 0..100 {
            let value = ["1", "FaLsE", "OFF", " true ", "", "unknown"][index % 6];
            xml.push_str("<w:p>");
            if kind == "paragraph" {
                xml.push_str("<w:pPr>");
                for name in [
                    "snapToGrid",
                    "adjustRightInd",
                    "widowControl",
                    "suppressAutoHyphens",
                    "contextualSpacing",
                    "pageBreakBefore",
                    "suppressLineNumbers",
                    "keepLines",
                    "keepNext",
                    "mirrorIndents",
                    "autoSpaceDE",
                    "autoSpaceDN",
                    "kinsoku",
                    "wordWrap",
                    "overflowPunct",
                    "topLinePunct",
                ] {
                    xml.push_str(&format!("<w:{name} w:val=\"{value}\"/>"));
                }
                xml.push_str("</w:pPr>");
            }
            xml.push_str("<w:r>");
            if kind == "run" {
                xml.push_str("<w:rPr>");
                for name in [
                    "b",
                    "i",
                    "bCs",
                    "iCs",
                    "caps",
                    "smallCaps",
                    "strike",
                    "dstrike",
                    "vanish",
                    "specVanish",
                    "shadow",
                    "outline",
                    "emboss",
                    "imprint",
                ] {
                    xml.push_str(&format!("<w:{name} w:val=\"{value}\"/>"));
                }
                xml.push_str("</w:rPr>");
            }
            xml.push_str("<w:t>Text</w:t></w:r></w:p>");
        }
        xml.push_str("</w:body></w:document>");
        fixtures.push((format!("{kind}=100"), xml));
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(include_bytes!(
        "../../../public/fixtures/yokohama_page6.docx"
    )))
    .expect("open formatting fixture");
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .expect("document part")
        .read_to_string(&mut xml)
        .expect("read document XML");
    fixtures.push(("yokohama_page6".to_owned(), xml));

    let mut group = c.benchmark_group("on_off_parse");
    for (name, xml) in fixtures {
        DocumentParser::new()
            .parse_document_xml(&xml)
            .expect("validate fixture");
        group.bench_function(name.as_str(), |b| {
            b.iter(|| {
                black_box(
                    DocumentParser::new()
                        .parse_document_xml(black_box(&xml))
                        .expect("parse document"),
                )
            });
        });
        group.bench_function(format!("{name}_with_raw_tree"), |b| {
            b.iter(|| {
                black_box(
                    DocumentParser::new()
                        .parse_document_xml_with_raw_tree(black_box(&xml))
                        .expect("parse document and XML"),
                )
            });
        });
    }
    group.finish();
}

fn bench_styles_parse(c: &mut Criterion) {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(include_bytes!(
        "../../../contract.docx"
    )))
    .expect("open contract");
    let mut contract_xml = String::new();
    archive
        .by_name("word/styles.xml")
        .expect("styles part")
        .read_to_string(&mut contract_xml)
        .expect("read styles XML");
    let mut fixtures = vec![("contract".to_owned(), contract_xml)];
    for kind in ["paragraph", "table"] {
        let mut xml = String::from(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
        );
        for id in 0..100 {
            xml.push_str(&format!(
                r#"<w:style w:type="{kind}" w:styleId="Style{id}"><w:name w:val="Style {id}"/><w:pPr><w:jc w:val="left"/><w:spacing w:after="160"/></w:pPr><w:rPr><w:rFonts w:ascii="Arial"/><w:sz w:val="22"/></w:rPr>"#,
            ));
            if kind == "table" {
                xml.push_str(r#"<w:tblPr><w:shd w:fill="AABBCC"/><w:tblCellMar><w:left w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr><w:tblStylePr w:type="firstRow"><w:rPr><w:b/></w:rPr><w:tcPr><w:shd w:fill="112233"/></w:tcPr></w:tblStylePr>"#);
            }
            xml.push_str("</w:style>");
        }
        xml.push_str("</w:styles>");
        fixtures.push((format!("{kind}=100"), xml));
    }
    let mut property_dense = String::from(
        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
    );
    for id in 0..100 {
        property_dense.push_str(&format!(
            r#"<w:style w:type="paragraph" w:styleId="Dense{id}"><w:name w:val="Dense {id}"/><w:pPr><w:jc w:val="both"/><w:spacing w:before="120" w:after="160" w:line="240" w:lineRule="auto"/><w:ind w:left="720" w:right="360" w:firstLine="240"/><w:keepNext w:val="1"/><w:keepLines w:val="0"/><w:outlineLvl w:val="2"/><w:tabs><w:tab w:val="left" w:pos="720" w:leader="dot"/></w:tabs><w:numPr><w:ilvl w:val="1"/><w:numId w:val="2"/></w:numPr></w:pPr><w:rPr><w:b w:val="1"/><w:i w:val="0"/><w:strike w:val="false"/><w:sz w:val="22"/><w:spacing w:val="20"/><w:color w:val="AABBCC"/><w:lang w:val="en-US" w:eastAsia="ja-JP"/><w:u w:val="single"/></w:rPr></w:style>"#,
        ));
    }
    property_dense.push_str("</w:styles>");
    fixtures.push(("property_dense=100".to_owned(), property_dense));
    for (name, suffix) in [
        ("fonts_empty=100", "/>"),
        ("fonts_start_end=100", "></w:rFonts>"),
    ] {
        let mut xml = String::from(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
        );
        for id in 0..100 {
            xml.push_str(&format!(
                r#"<w:style w:type="paragraph" w:styleId="Fonts{id}"><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Calibri" w:eastAsia="ＭＳ 明朝" w:cs="Times New Roman" w:asciiTheme="minorHAnsi" w:eastAsiaTheme="minorEastAsia" w:cstheme="minorBidi" w:hint="eastAsia" w:unused="ignored"{suffix}</w:rPr></w:style>"#,
            ));
        }
        xml.push_str("</w:styles>");
        fixtures.push((name.to_owned(), xml));
    }
    for unknown_count in [3, 9] {
        let attributes = (0..unknown_count)
            .map(|id| format!(r#"w:unused{id}="ignored" "#))
            .collect::<String>();
        let mut xml = String::from(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
        );
        for id in 0..100 {
            xml.push_str(&format!(
                r#"<w:style {attributes}w:type="paragraph" w:styleId="Late{id}"><w:name {attributes}w:val="Late {id}"/><w:pPr><w:jc {attributes}w:val="left"/></w:pPr><w:rPr><w:color {attributes}w:val="AABBCC"/><w:b {attributes}w:val="0"/></w:rPr></w:style>"#,
            ));
        }
        xml.push_str("</w:styles>");
        fixtures.push((format!("attributes_late={unknown_count}x100"), xml));
    }
    let mut value_conversion = String::from(
        r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
    );
    for id in 0..100 {
        value_conversion.push_str(&format!(
            r#"<w:style w:type="paragraph" w:styleId="Values{id}"><w:pPr><w:spacing w:before="120" w:after="160" w:line="480" w:lineRule="exactly"/><w:keepNext w:val=" OFF "/><w:keepLines w:val="FaLsE"/><w:tabs><w:tab w:val="left" w:pos="720" w:leader="dot"/><w:tab w:val="right" w:pos="1440" w:leader="underscore"/></w:tabs></w:pPr><w:rPr><w:b w:val="0"/><w:i w:val="off"/><w:color w:val="aAbBcC"/></w:rPr></w:style>"#,
        ));
    }
    value_conversion.push_str("</w:styles>");
    fixtures.push(("value_conversion=100".to_owned(), value_conversion));
    let mut group = c.benchmark_group("styles_parse");
    for (name, xml) in fixtures {
        StylesPart::from_xml(&xml).expect("validate styles fixture");
        group.bench_function(name.as_str(), |b| {
            b.iter(|| black_box(StylesPart::from_xml(black_box(&xml)).expect("parse styles")));
        });
        let mut styles = StylesPart::from_xml(&xml).expect("parse styles");
        group.bench_function(format!("{name}_reuse"), |b| {
            b.iter(|| styles.parse(black_box(&xml)).expect("reparse styles"));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_props,
    bench_docx_xml_path_updates,
    bench_word_namespaces,
    bench_outline,
    bench_run_font_parse,
    bench_paragraph_style_resolution,
    bench_numbering_parse,
    bench_docx_xml_parse,
    bench_on_off_parse,
    bench_styles_parse
);
criterion_main!(benches);
