use jade_docx_document_core::{
    commands::{CommandContext, CommandExecutor},
    headless_api::{apply_headless_commands_with_executor_to_document, HeadlessApiApplyError},
    word_document::WordDocument,
};
use jade_docx_headless_editor::{apply_headless_commands, HeadlessCommand, HeadlessEditRequest};
use proptest::prelude::*;
use std::io::{Cursor, Read, Write};
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

fn part(bytes: &[u8], name: &str) -> String {
    let mut zip = ZipArchive::new(Cursor::new(bytes)).expect("package");
    let mut text = String::new();
    zip.by_name(name)
        .expect("part")
        .read_to_string(&mut text)
        .expect("xml");
    text
}
fn package(body: &str) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    let document = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}</w:body></w:document>"#
    );
    let comments = format!(r#"<w:comments xmlns:w="{W}">{}</w:comments>"#, (7..11).map(|id| format!(r#"<w:comment w:id="{id}" w:author="Original synthetic reviewer" w:initials="OS" w:date="2020-01-01T00:00:00Z"><w:p><w:r><w:t>Synthetic review</w:t></w:r></w:p></w:comment>"#)).collect::<String>());
    for (name, text) in [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        ("word/document.xml", document.as_str()),
        ("word/comments.xml", comments.as_str()),
        (
            "word/_rels/document.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rIdComments" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/></Relationships>"#,
        ),
    ] {
        zip.start_file(name, SimpleFileOptions::default())
            .expect("entry");
        zip.write_all(text.as_bytes()).expect("write");
    }
    zip.finish().expect("zip").into_inner()
}
fn insert(bytes: &[u8], paragraph: usize, offset: usize) -> Vec<u8> {
    apply_headless_commands(
        bytes,
        &HeadlessEditRequest {
            commands: vec![
                HeadlessCommand::StartChangeTracking {
                    author: Some("Synthetic".into()),
                    initials: Some("S".into()),
                    date: Some("2026-10-30T00:00:00Z".into()),
                    date_utc: None,
                },
                HeadlessCommand::InsertText {
                    paragraph,
                    offset,
                    text: "検証".into(),
                },
            ],
        },
    )
    .expect("synthetic edit")
}
fn code(xml: &str) -> Vec<String> {
    let doc = roxmltree::Document::parse(xml).expect("xml");
    doc.descendants()
        .filter(|n| n.is_element() && matches!(n.tag_name().name(), "fldChar" | "instrText"))
        .map(|n| xml[n.range()].to_owned())
        .collect()
}
#[test]
fn spanning_toc_tracked_insertion_preserves_code() {
    let bytes = package(
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin" w:dirty="true"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>First</w:t></w:r></w:p><w:p><w:r><w:t>Second</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
    );
    let output = insert(&bytes, 0, 0);
    assert_eq!(
        code(&part(&output, "word/document.xml")),
        code(&part(&bytes, "word/document.xml"))
    );
}

// Edit plain text beside atomic fields and inside a spanning TOC result.
// Semantic IF results are immutable cached text, absent from public edit offsets.
// Replacing a whole field, unlinking fields and splitting paragraphs have different
// contracts and are deliberately outside this state machine.
#[derive(Clone, Debug)]
struct Shape {
    paragraphs: usize,
    depth: usize,
    split: usize,
    operand: bool,
    reviews: bool,
    unicode: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Review {
    Plain,
    Inserted,
    Deleted,
}
#[derive(Clone, Debug)]
enum Event {
    Text {
        region: usize,
        chars: Vec<(char, Review)>,
    },
    Fixed(String),
    Cached(String),
    Deleted(String),
    Token(String),
}
#[derive(Clone, Debug)]
struct Model {
    paragraphs: Vec<Vec<Event>>,
}
#[derive(Debug, PartialEq, Eq)]
struct Observation {
    text: Vec<String>,
    // Paragraph and byte position make a reordered/moved code token observable,
    // even if all the fldChar counts and the final text happen to agree.
    tokens: Vec<(usize, usize, String)>,
    reviews: Vec<(usize, String)>,
    paragraphs: Vec<String>,
}
fn canonical(node: roxmltree::Node<'_, '_>) -> String {
    if node.is_text() {
        return format!("{:?}", node.text().unwrap_or_default());
    }
    let mut attributes: Vec<_> = node
        .attributes()
        .filter(|a| {
            // Redundant xml:space on a text with no surrounding whitespace is
            // serialization trivia; instruction attributes remain exact.
            !(node.has_tag_name((W, "t"))
                && a.namespace() == Some("http://www.w3.org/XML/1998/namespace")
                && a.name() == "space"
                && node.text().is_some_and(|text| text.trim() == text))
        })
        .map(|a| (a.namespace(), a.name(), a.value()))
        .collect();
    attributes.sort_unstable();
    format!(
        "{:?}:{:?}{attributes:?}[{}]",
        node.tag_name().namespace(),
        node.tag_name().name(),
        node.children()
            .filter(|n| n.is_element() || n.is_text())
            .map(canonical)
            .collect::<String>()
    )
}
fn signature(xml: &str) -> String {
    let source = format!(
        r#"<root xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:xml="http://www.w3.org/XML/1998/namespace">{xml}</root>"#
    );
    canonical(
        roxmltree::Document::parse(&source)
            .expect("synthetic token")
            .root_element()
            .first_element_child()
            .expect("token"),
    )
}
fn observe(xml: &str) -> Observation {
    fn walk(
        node: roxmltree::Node<'_, '_>,
        p: usize,
        fields: &mut Vec<bool>,
        out: &mut Observation,
    ) {
        if !node.is_element() {
            return;
        }
        let name = node.tag_name().name();
        let at = out.text[p].len();
        if name == "fldChar"
            || name == "instrText"
            || name.starts_with("commentRange")
            || name == "commentReference"
            || (name == "t" && fields.contains(&true))
        {
            out.tokens.push((p, at, canonical(node)));
        }
        if name == "fldChar" {
            match node
                .attributes()
                .find(|a| a.name() == "fldCharType")
                .map(|a| a.value())
            {
                Some("begin") => fields.push(true),
                Some("separate") => *fields.last_mut().expect("separate needs begin") = false,
                Some("end") => {
                    fields.pop().expect("end needs begin");
                }
                _ => panic!("field token"),
            }
        }
        if name == "t"
            && !fields.contains(&true)
            && !node
                .ancestors()
                .any(|n| n.has_tag_name((W, "del")) || n.has_tag_name((W, "moveFrom")))
        {
            out.text[p].push_str(node.text().unwrap_or_default());
        }
        if matches!(name, "ins" | "del")
            && node
                .attributes()
                .any(|a| a.name() == "author" && a.value() == "Original synthetic reviewer")
        {
            out.reviews.push((p, canonical(node)));
        }
        for child in node.children().filter(|n| n.is_element()) {
            walk(child, p, fields, out);
        }
    }
    let doc = roxmltree::Document::parse(xml).expect("valid saved XML");
    let mut out = Observation {
        text: vec![],
        tokens: vec![],
        reviews: vec![],
        paragraphs: vec![],
    };
    let mut fields = Vec::new();
    for (p, paragraph) in doc
        .descendants()
        .filter(|n| n.has_tag_name((W, "p")))
        .enumerate()
    {
        out.text.push(String::new());
        out.paragraphs.push(canonical(paragraph));
        walk(paragraph, p, &mut fields, &mut out);
    }
    assert!(
        fields.is_empty(),
        "unclosed field across paragraph boundaries"
    );
    out
}
const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
fn token(body: &mut String, events: &mut Vec<Event>, xml: &str) {
    events.push(Event::Token(signature(xml)));
    body.push_str(&format!("<w:r>{xml}</w:r>"));
}
fn instruction(body: &mut String, events: &mut Vec<Event>, text: &str, split: usize) {
    let chars: Vec<_> = text.chars().collect();
    for fragment in chars.chunks(split) {
        token(
            body,
            events,
            &format!(
                r#"<w:instrText xml:space="preserve">{}</w:instrText>"#,
                escape(&fragment.iter().collect::<String>())
            ),
        );
    }
}
fn text(body: &mut String, events: &mut Vec<Event>, region: usize, value: &str) {
    events.push(Event::Text {
        region,
        chars: value.chars().map(|c| (c, Review::Plain)).collect(),
    });
    body.push_str(&format!(
        r#"<w:r><w:t xml:space="preserve">{}</w:t></w:r>"#,
        escape(value)
    ));
}
fn synthetic(shape: &Shape) -> (Vec<u8>, Model) {
    let mut body = String::new();
    let mut paragraphs = Vec::new();
    for p in 0..shape.paragraphs {
        body.push_str("<w:p><w:pPr><w:keepNext/></w:pPr>");
        let mut events = Vec::new();
        text(&mut body, &mut events, 0, &format!("L{}", shape.unicode));
        if p == 0 {
            token(
                &mut body,
                &mut events,
                r#"<w:fldChar w:fldCharType="begin" w:dirty="true"/>"#,
            );
            instruction(&mut body, &mut events, r#" TOC \o "1-3" \h "#, shape.split);
            token(
                &mut body,
                &mut events,
                r#"<w:fldChar w:fldCharType="separate"/>"#,
            );
        }
        for d in 0..shape.depth {
            token(
                &mut body,
                &mut events,
                r#"<w:fldChar w:fldCharType="begin" w:fldLock="true"/>"#,
            );
            instruction(
                &mut body,
                &mut events,
                &if shape.operand {
                    " IF ".to_owned()
                } else {
                    format!(r#" IF "synthetic_{p}_{d}" = "yes" "A" "B" "#)
                },
                shape.split,
            );
            if shape.operand {
                token(
                    &mut body,
                    &mut events,
                    r#"<w:fldChar w:fldCharType="begin"/>"#,
                );
                instruction(&mut body, &mut events, " MERGEFIELD Operand ", shape.split);
                token(
                    &mut body,
                    &mut events,
                    r#"<w:fldChar w:fldCharType="separate"/>"#,
                );
                token(
                    &mut body,
                    &mut events,
                    r#"<w:t xml:space="preserve">hidden operand😀</w:t>"#,
                );
                token(
                    &mut body,
                    &mut events,
                    r#"<w:fldChar w:fldCharType="end"/>"#,
                );
                instruction(&mut body, &mut events, r#" = "yes" "A" "B" "#, shape.split);
            }
            token(
                &mut body,
                &mut events,
                r#"<w:fldChar w:fldCharType="separate"/>"#,
            );
        }
        let cached = format!("result{}end", shape.unicode);
        body.push_str(&format!(
            r#"<w:r><w:t xml:space="preserve">{}</w:t></w:r>"#,
            escape(&cached)
        ));
        events.push(Event::Cached(cached));
        for _ in 0..shape.depth {
            token(
                &mut body,
                &mut events,
                r#"<w:fldChar w:fldCharType="end"/>"#,
            );
        }
        if p == shape.paragraphs - 1 {
            token(
                &mut body,
                &mut events,
                r#"<w:fldChar w:fldCharType="end"/>"#,
            );
        }
        text(
            &mut body,
            &mut events,
            1,
            &format!("body{}end", shape.unicode),
        );
        text(&mut body, &mut events, 2, "suffix");
        if shape.reviews {
            // An existing comment and both revision kinds share the edited
            // paragraph, but their own text is outside the editable regions.
            let start = format!(r#"<w:commentRangeStart w:id="{}"/>"#, 7 + p);
            let end = format!(r#"<w:commentRangeEnd w:id="{}"/>"#, 7 + p);
            let reference = format!(r#"<w:commentReference w:id="{}"/>"#, 7 + p);
            body.push_str(&start);
            events.push(Event::Token(signature(&start)));
            body.push_str(&format!(r#"<w:ins w:id="{}" w:author="Original synthetic reviewer" w:date="2020-01-01T00:00:00Z"><w:r><w:t>original</w:t></w:r></w:ins>"#, 701 + 2*p));
            events.push(Event::Fixed("original".into()));
            body.push_str(&format!(r#"<w:del w:id="{}" w:author="Original synthetic reviewer" w:date="2020-01-01T00:00:00Z"><w:r><w:delText>removed</w:delText></w:r></w:del>"#, 702 + 2*p));
            events.push(Event::Deleted("removed".into()));
            body.push_str(&end);
            events.push(Event::Token(signature(&end)));
            body.push_str(&format!("<w:r>{reference}</w:r>"));
            events.push(Event::Token(signature(&reference)));
        }
        body.push_str("</w:p>");
        paragraphs.push(events);
    }
    (package(&body), Model { paragraphs })
}
struct RegionLocation {
    api_base: usize,
    xml_base: usize,
    // Unit index, public byte offset, visible XML byte offset.
    boundaries: Vec<(usize, usize, usize)>,
}
impl Model {
    fn region(&self, p: usize, region: usize) -> RegionLocation {
        let (mut api_base, mut xml_base) = (0, 0);
        for event in &self.paragraphs[p] {
            match event {
                Event::Text { region: id, chars } => {
                    let (mut api, mut xml) = (0, 0);
                    let mut boundaries = Vec::new();
                    for (i, (c, deleted)) in chars.iter().enumerate() {
                        if *deleted != Review::Deleted {
                            boundaries.push((i, api, xml));
                            xml += c.len_utf8();
                        }
                        api += c.len_utf8();
                    }
                    boundaries.push((chars.len(), api, xml));
                    if *id == region {
                        return RegionLocation {
                            api_base,
                            xml_base,
                            boundaries,
                        };
                    }
                    api_base += api;
                    xml_base += xml;
                }
                Event::Fixed(value) => {
                    api_base += value.len();
                    xml_base += value.len();
                }
                Event::Cached(value) => xml_base += value.len(),
                Event::Deleted(value) => api_base += value.len(),
                Event::Token(_) => {}
            }
        }
        unreachable!("generated region")
    }
    fn edit(
        &mut self,
        p: usize,
        region: usize,
        start: usize,
        end: usize,
        text: &str,
        tracked: bool,
    ) {
        for event in &mut self.paragraphs[p] {
            if let Event::Text { region: id, chars } = event {
                if *id != region {
                    continue;
                }
                if tracked {
                    let retained: Vec<_> = chars[start..end]
                        .iter()
                        .filter(|(_, review)| *review != Review::Inserted)
                        .map(|(c, _)| (*c, Review::Deleted))
                        .collect();
                    chars.splice(
                        start..end,
                        text.chars().map(|c| (c, Review::Inserted)).chain(retained),
                    );
                } else {
                    // Deleted revisions are protected even while tracking is
                    // off. A range crossing them removes only live text.
                    let retained: Vec<_> = chars[start..end]
                        .iter()
                        .copied()
                        .filter(|(_, review)| *review == Review::Deleted)
                        .collect();
                    chars.splice(
                        start..end,
                        text.chars().map(|c| (c, Review::Plain)).chain(retained),
                    );
                }
                return;
            }
        }
    }
    fn editable_text(&self, p: usize) -> String {
        self.paragraphs[p]
            .iter()
            .map(|event| match event {
                Event::Text { chars, .. } => chars.iter().map(|(c, _)| c).collect(),
                Event::Fixed(value) | Event::Deleted(value) => value.clone(),
                _ => String::new(),
            })
            .collect()
    }
    fn expected(&self) -> (Vec<String>, Vec<(usize, usize, String)>) {
        let mut texts = Vec::new();
        let mut tokens = Vec::new();
        for (p, events) in self.paragraphs.iter().enumerate() {
            let mut text = String::new();
            for event in events {
                match event {
                    Event::Text { chars, .. } => text.extend(
                        chars
                            .iter()
                            .filter(|(_, deleted)| *deleted != Review::Deleted)
                            .map(|(c, _)| c),
                    ),
                    Event::Fixed(value) | Event::Cached(value) => text.push_str(value),
                    Event::Deleted(_) => {}
                    Event::Token(code) => tokens.push((p, text.len(), code.clone())),
                }
            }
            texts.push(text);
        }
        (texts, tokens)
    }
}
#[derive(Clone, Debug)]
enum Op {
    Insert {
        paragraph: usize,
        region: usize,
        boundary: u8,
        text: String,
        tracked: bool,
    },
    Erase {
        paragraph: usize,
        region: usize,
        tracked: bool,
    },
    Reload,
    InvalidParagraph,
    InvalidOffset,
    InsideUnicode,
    Empty,
}
fn shape() -> impl Strategy<Value = Shape> {
    (
        2usize..5,
        1usize..4,
        1usize..12,
        any::<bool>(),
        any::<bool>(),
        prop::sample::select(vec!["", "漢", "😀", "e\u{301}", "<>&😀"]),
    )
        .prop_map(
            |(paragraphs, depth, split, operand, reviews, unicode)| Shape {
                paragraphs,
                depth,
                split,
                operand,
                reviews,
                unicode: unicode.into(),
            },
        )
}
fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        7 => (0usize..4, 0usize..3, any::<u8>(), prop::sample::select(vec!["x", "検証", "😀", "e\u{301}", "<&>", " "]), any::<bool>())
            .prop_map(|(paragraph, region, boundary, text, tracked)| Op::Insert { paragraph, region, boundary, text: text.into(), tracked }),
        3 => (0usize..4, 0usize..3, any::<bool>()).prop_map(|(paragraph, region, tracked)| Op::Erase { paragraph, region, tracked }),
        2 => Just(Op::Reload), 1 => Just(Op::InvalidParagraph), 1 => Just(Op::InvalidOffset),
        1 => Just(Op::InsideUnicode), 1 => Just(Op::Empty),
    ]
}
fn tracking(tracked: bool) -> HeadlessCommand {
    if tracked {
        HeadlessCommand::StartChangeTracking {
            author: Some("Synthetic edit".into()),
            initials: Some("SE".into()),
            date: Some("2026-10-30T00:00:00Z".into()),
            date_utc: None,
        }
    } else {
        HeadlessCommand::StopChangeTracking
    }
}
fn exercise(shape: Shape, ops: Vec<Op>) -> Result<(), TestCaseError> {
    let (mut bytes, mut model) = synthetic(&shape);
    let comments = part(&bytes, "word/comments.xml");
    let relationships = part(&bytes, "word/_rels/document.xml.rels");
    let baseline = observe(&part(&bytes, "word/document.xml"));
    let (text, tokens) = model.expected();
    prop_assert_eq!(&baseline.text, &text, "generator text");
    prop_assert_eq!(&baseline.tokens, &tokens, "generator code");
    for (step, op) in ops.iter().enumerate() {
        let before = observe(&part(&bytes, "word/document.xml"));
        let mut changed = None;
        let mut edit = None;
        let mut rejection = None;
        let mut commands = Vec::new();
        match op {
            Op::Insert {
                paragraph,
                region,
                boundary,
                text,
                tracked,
            } => {
                let p = paragraph % shape.paragraphs;
                let location = model.region(p, *region);
                // At the byte shared by an atomic cached field and following
                // text, insertion belongs before the field. Use the following
                // region's interior so its intended visible position is unique.
                let first = usize::from(*region == 1);
                let (unit, at, xml_at) = location.boundaries
                    [first + usize::from(*boundary) % (location.boundaries.len() - first)];
                commands.push(tracking(*tracked));
                commands.push(HeadlessCommand::InsertText {
                    paragraph: p,
                    offset: location.api_base + at,
                    text: text.clone(),
                });
                edit = Some((
                    p,
                    location.xml_base + xml_at,
                    location.xml_base + xml_at,
                    text.len(),
                ));
                model.edit(p, *region, unit, unit, text, *tracked);
                changed = Some(p);
            }
            Op::Erase {
                paragraph,
                region,
                tracked,
            } => {
                let p = paragraph % shape.paragraphs;
                let location = model.region(p, *region);
                // Keep at least two visible scalars, avoiding whole-field or
                // whole-region deletion while shrinking remains total.
                let (start, end) = if location.boundaries.len() > 3 {
                    (location.boundaries[1], location.boundaries[2])
                } else {
                    (location.boundaries[0], location.boundaries[0])
                };
                commands.push(tracking(*tracked));
                commands.push(HeadlessCommand::ReplaceRange {
                    start_paragraph: p,
                    end_paragraph: p,
                    start_offset: location.api_base + start.1,
                    end_offset: location.api_base + end.1,
                    text: String::new(),
                });
                edit = Some((p, location.xml_base + start.2, location.xml_base + end.2, 0));
                model.edit(p, *region, start.0, end.0, "", *tracked);
                changed = Some(p);
            }
            Op::InvalidParagraph => {
                rejection = Some(format!("paragraph {} does not exist", shape.paragraphs));
                commands.push(HeadlessCommand::InsertText {
                    paragraph: shape.paragraphs,
                    offset: 0,
                    text: "x".into(),
                });
            }
            Op::InvalidOffset => {
                let len = model.editable_text(0).len();
                rejection = Some(format!(
                    "offset {} is past the end of paragraph 0, which is {len} bytes long",
                    len + 1
                ));
                commands.push(HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: len + 1,
                    text: "x".into(),
                });
            }
            Op::InsideUnicode => {
                let invalid = model
                    .editable_text(0)
                    .char_indices()
                    .find(|(_, c)| c.len_utf8() > 1)
                    .map(|(i, _)| i + 1);
                rejection = invalid.map(|offset| format!("offset {offset} is inside a character in paragraph 0; offsets are UTF-8 byte offsets, and a search_text result gives valid ones"));
                commands.push(HeadlessCommand::InsertText {
                    paragraph: 0,
                    offset: invalid.unwrap_or(0),
                    text: String::new(),
                });
            }
            Op::Empty => commands.push(HeadlessCommand::InsertText {
                paragraph: 0,
                offset: 0,
                text: String::new(),
            }),
            Op::Reload => commands.push(HeadlessCommand::StopChangeTracking),
        }
        // This facade performs the real parse -> command -> export boundary on
        // every step; the next operation necessarily edits the reloaded package.
        let request = HeadlessEditRequest { commands };
        let outcome = apply_headless_commands(&bytes, &request);
        if let Some(expected) = rejection {
            let Err(jade_docx_headless_editor::HeadlessEditError::Apply(error)) = outcome else {
                return Err(TestCaseError::fail(format!(
                    "step {step}, {op:?}: expected boundary rejection"
                )));
            };
            assert_rejection(error, &expected)?;
            // Also exercise the mutable public API: a rejected command must not
            // corrupt the in-memory package that its caller continues using.
            let mut doc = futures::executor::block_on(
                jade_docx_document_core::word_document::WordDocument::load(
                    &bytes,
                    &Default::default(),
                ),
            )
            .expect("synthetic load");
            let error = jade_docx_document_core::headless_api::apply_headless_commands_to_document(
                &mut doc, &request,
            )
            .expect_err("invalid boundary");
            assert_rejection(error, &expected)?;
            bytes = doc.export_docx().expect("export after rejection");
        } else {
            bytes =
                outcome.map_err(|e| TestCaseError::fail(format!("step {step}, {op:?}: {e}")))?;
        }
        prop_assert_eq!(
            part(&bytes, "word/comments.xml"),
            comments.clone(),
            "comment part"
        );
        prop_assert_eq!(
            part(&bytes, "word/_rels/document.xml.rels"),
            relationships.clone(),
            "relationships"
        );
        let after = observe(&part(&bytes, "word/document.xml"));
        let loaded = futures::executor::block_on(WordDocument::load(&bytes, &Default::default()))
            .expect("reload edit offsets");
        for p in 0..shape.paragraphs {
            let actual: String = loaded
                .paragraph(p)
                .expect("paragraph")
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect();
            prop_assert_eq!(
                actual,
                model.editable_text(p),
                "step {}, {:?}: public offset text in paragraph {}",
                step,
                op,
                p
            );
        }
        let (text, _) = model.expected();
        prop_assert_eq!(
            after.tokens.len(),
            before.tokens.len(),
            "step {}, {:?}: field/anchor count",
            step,
            op
        );
        for (index, ((p, at, token), (old_p, old_at, old_token))) in
            after.tokens.iter().zip(&before.tokens).enumerate()
        {
            prop_assert_eq!(
                (p, token),
                (old_p, old_token),
                "step {}, {:?}: token {} topology/content",
                step,
                op,
                index
            );
            let allowed = match edit {
                Some((edited_p, start, end, inserted)) if *p == edited_p => {
                    if *old_at < start {
                        *at == *old_at
                    } else if *old_at > end {
                        *at == old_at - (end - start) + inserted
                    }
                    // The public byte-offset API does not expose marker affinity.
                    // At a shared boundary either side of inserted text is valid.
                    else {
                        *at == start || *at == start + inserted
                    }
                }
                _ => at == old_at,
            };
            prop_assert!(allowed, "step {step}, {op:?}: token {index} moved from {old_at} to {at} outside edit {edit:?}");
        }
        prop_assert_eq!(&after.text, &text, "step {}, {:?}: visible text", step, op);
        prop_assert_eq!(
            &after.reviews,
            &baseline.reviews,
            "step {}, {:?}: existing revisions",
            step,
            op
        );
        for p in 0..shape.paragraphs {
            if Some(p) != changed {
                prop_assert_eq!(
                    &after.paragraphs[p],
                    &before.paragraphs[p],
                    "step {}, {:?}: untouched paragraph {}",
                    step,
                    op,
                    p
                );
            }
        }
    }
    Ok(())
}
fn assert_rejection(error: HeadlessApiApplyError, expected: &str) -> Result<(), TestCaseError> {
    let HeadlessApiApplyError::Command { index, source } = error else {
        return Err(TestCaseError::fail("expected command error"));
    };
    prop_assert_eq!(index, 0, "validation must identify the rejected command");
    let jade_docx_document_core::commands::CommandError::Custom(message) = source else {
        return Err(TestCaseError::fail("expected boundary error"));
    };
    prop_assert_eq!(message, expected);
    Ok(())
}

#[derive(Clone, Debug)]
enum HistoryOp {
    Insert {
        paragraph: usize,
        region: usize,
        boundary: u8,
        tracked: bool,
    },
    Undo,
    Redo,
    Reload,
}
fn history_op() -> impl Strategy<Value = HistoryOp> {
    prop_oneof![
        5 => (0usize..4, 0usize..3, any::<u8>(), any::<bool>())
            .prop_map(|(paragraph, region, boundary, tracked)| HistoryOp::Insert { paragraph, region, boundary, tracked }),
        3 => Just(HistoryOp::Undo), 3 => Just(HistoryOp::Redo), 1 => Just(HistoryOp::Reload),
    ]
}
fn exercise_history(shape: Shape, ops: Vec<HistoryOp>) -> Result<(), TestCaseError> {
    let (bytes, mut model) = synthetic(&shape);
    let baseline = observe(&part(&bytes, "word/document.xml"));
    let comments = part(&bytes, "word/comments.xml");
    let relationships = part(&bytes, "word/_rels/document.xml.rels");
    let load = |bytes: &[u8]| {
        futures::executor::block_on(WordDocument::load(bytes, &Default::default()))
            .expect("synthetic load")
    };
    let mut doc = load(&bytes);
    let mut executor = CommandExecutor::new();
    let mut selection = None;
    // Model snapshots predict the text independently. XML snapshots additionally
    // assert the inverse-operation contract, including marker affinity and IDs.
    let mut undo: Vec<(Model, Observation)> = Vec::new();
    let mut redo: Vec<(Model, Observation)> = Vec::new();
    for (step, op) in ops.iter().enumerate() {
        let before = observe(&part(
            &doc.export_docx().expect("save before operation"),
            "word/document.xml",
        ));
        let mut restored = None;
        match op {
            HistoryOp::Insert {
                paragraph,
                region,
                boundary,
                tracked,
            } => {
                let p = paragraph % shape.paragraphs;
                let location = model.region(p, *region);
                let first = usize::from(*region == 1);
                let (unit, at, _) = location.boundaries
                    [first + usize::from(*boundary) % (location.boundaries.len() - first)];
                undo.push((model.clone(), before));
                redo.clear();
                let text = "検証😀";
                model.edit(p, *region, unit, unit, text, *tracked);
                // A logical authoring action groups the session selection and
                // content edit, so redo restores the original tracking mode.
                executor.begin_group();
                selection = apply_headless_commands_with_executor_to_document(
                    &mut doc,
                    &mut executor,
                    selection,
                    &HeadlessEditRequest {
                        commands: vec![
                            tracking(*tracked),
                            HeadlessCommand::InsertText {
                                paragraph: p,
                                offset: location.api_base + at,
                                text: text.into(),
                            },
                        ],
                    },
                )
                .map_err(|e| TestCaseError::fail(format!("step {step}, {op:?}: {e}")))?;
                executor.end_group();
            }
            HistoryOp::Undo | HistoryOp::Redo => {
                let (from, to) = if matches!(op, HistoryOp::Undo) {
                    (&mut undo, &mut redo)
                } else {
                    (&mut redo, &mut undo)
                };
                let expected = from.pop();
                let mut context = CommandContext::new(&mut doc, &mut selection);
                let actual = if matches!(op, HistoryOp::Undo) {
                    executor.undo(&mut context)
                } else {
                    executor.redo(&mut context)
                }
                .expect("history command");
                prop_assert_eq!(
                    actual,
                    expected.is_some(),
                    "step {}, {:?}: history availability",
                    step,
                    op
                );
                if let Some((previous, snapshot)) = expected {
                    to.push((model.clone(), before));
                    model = previous;
                    restored = Some(snapshot);
                } else {
                    restored = Some(before);
                }
            }
            HistoryOp::Reload => {
                doc = load(&doc.export_docx().expect("save for reload"));
                executor = CommandExecutor::new();
                selection = None;
                undo.clear();
                redo.clear();
                restored = Some(before);
            }
        }
        let bytes = doc.export_docx().expect("save after operation");
        let after = observe(&part(&bytes, "word/document.xml"));
        let (text, tokens) = model.expected();
        prop_assert_eq!(&after.text, &text, "step {}, {:?}: history text", step, op);
        prop_assert_eq!(
            after
                .tokens
                .iter()
                .map(|(p, _, token)| (p, token))
                .collect::<Vec<_>>(),
            tokens
                .iter()
                .map(|(p, _, token)| (p, token))
                .collect::<Vec<_>>(),
            "step {}, {:?}: history field topology",
            step,
            op
        );
        prop_assert_eq!(&after.reviews, &baseline.reviews, "existing reviews");
        if let Some(snapshot) = restored {
            prop_assert_eq!(&after, &snapshot, "step {}, {:?}: restored XML", step, op);
        }
        prop_assert_eq!(part(&bytes, "word/comments.xml"), comments.clone());
        prop_assert_eq!(
            part(&bytes, "word/_rels/document.xml.rels"),
            relationships.clone()
        );
        prop_assert_eq!(
            executor.undo_depth(),
            undo.len(),
            "step {}, {:?}: undo depth",
            step,
            op
        );
        prop_assert_eq!(
            executor.can_redo(),
            !redo.is_empty(),
            "step {}, {:?}: redo availability",
            step,
            op
        );
        let reloaded = load(&bytes).export_docx().expect("save reloaded document");
        prop_assert_eq!(
            observe(&part(&reloaded, "word/document.xml")),
            after,
            "step {}, {:?}: reload stability",
            step,
            op
        );
    }
    Ok(())
}

#[test]
fn nested_fields_survive_boundary_edits_and_history_branches() {
    let shape = Shape {
        paragraphs: 4,
        depth: 3,
        split: 1,
        operand: true,
        reviews: true,
        unicode: "<>&😀e\u{301}".into(),
    };
    let mut edits = Vec::new();
    for paragraph in 0..4 {
        for region in 0..3 {
            edits.push(Op::Insert {
                paragraph,
                region,
                boundary: 0,
                text: "検証😀".into(),
                tracked: true,
            });
            edits.push(Op::Erase {
                paragraph,
                region,
                tracked: true,
            });
            edits.push(Op::Reload);
        }
    }
    edits.extend([
        Op::InvalidParagraph,
        Op::InvalidOffset,
        Op::InsideUnicode,
        Op::Empty,
    ]);
    exercise(shape.clone(), edits).expect("boundary edit regression");
    exercise(
        Shape {
            unicode: "e\u{301}".into(),
            ..shape.clone()
        },
        vec![
            Op::Erase {
                paragraph: 0,
                region: 0,
                tracked: true,
            },
            Op::Insert {
                paragraph: 0,
                region: 0,
                boundary: 112,
                text: "検証".into(),
                tracked: false,
            },
            Op::Insert {
                paragraph: 0,
                region: 0,
                boundary: 110,
                text: "😀".into(),
                tracked: false,
            },
            Op::Erase {
                paragraph: 0,
                region: 0,
                tracked: false,
            },
            Op::Insert {
                paragraph: 0,
                region: 1,
                boundary: 0,
                text: "x".into(),
                tracked: false,
            },
        ],
    )
    .expect("protected deleted text still contributes to API offsets");
    exercise_history(
        shape,
        vec![
            HistoryOp::Undo,
            HistoryOp::Redo,
            HistoryOp::Insert {
                paragraph: 0,
                region: 0,
                boundary: 0,
                tracked: true,
            },
            HistoryOp::Insert {
                paragraph: 3,
                region: 1,
                boundary: 1,
                tracked: false,
            },
            HistoryOp::Undo,
            HistoryOp::Undo,
            HistoryOp::Redo,
            HistoryOp::Redo,
            HistoryOp::Undo,
            HistoryOp::Insert {
                paragraph: 1,
                region: 2,
                boundary: 0,
                tracked: true,
            },
            HistoryOp::Redo,
            HistoryOp::Undo,
            HistoryOp::Redo,
            HistoryOp::Reload,
            HistoryOp::Undo,
            HistoryOp::Redo,
        ],
    )
    .expect("history branch regression");
}
proptest! {
    #![proptest_config(ProptestConfig { cases: 96, max_shrink_iters: 4096, failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct("tests/complex_fields.proptest-regressions"))), ..ProptestConfig::default() })]
    #[test]
    fn property_complex_fields_survive_edit_export_reload(shape in shape(), ops in prop::collection::vec(op(), 1..=40)) {
        exercise(shape, ops)?;
    }
    #[test]
    fn property_complex_fields_survive_undo_redo(shape in shape(), ops in prop::collection::vec(history_op(), 1..=40)) {
        exercise_history(shape, ops)?;
    }
}
