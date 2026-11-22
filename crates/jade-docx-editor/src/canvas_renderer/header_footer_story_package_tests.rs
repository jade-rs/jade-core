//! Header and footer stories of a loaded package with many sections that
//! link to the previous one: every section shows the first section's
//! letterhead (a table and a picture) and its three-line footer, which
//! reaches into the bottom margin, so each page measures its footer before
//! filling the body.

use super::recording_host::RecordingHost;
use super::*;
use crate::canvas_view::EditingMode;
use crate::word_document::WordDocument;
use jade_painter::recording::{Op, RecordingPainter};
use std::io::Write;

const NAMESPACES: &str = concat!(
    r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" "#,
    r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
    r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
    r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
    r#"xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture""#,
);

/// A 1x1 PNG padded to a logo's size, so its data URI is not trivial.
fn logo_png() -> Vec<u8> {
    let mut png = general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==")
        .expect("png");
    png.resize(png.len() + 24 * 1024, 0);
    png
}

fn header_xml() -> String {
    let cell = |text: &str| format!("<w:tc><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:tc>");
    let row = |a: &str, b: &str| format!("<w:tr>{}{}</w:tr>", cell(a), cell(b));
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8"?><w:hdr {ns}>"#,
            "<w:p><w:r><w:t>Letterhead</w:t></w:r></w:p>",
            r#"<w:tbl><w:tblGrid><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/></w:tblGrid>{r1}{r2}</w:tbl>"#,
            r#"<w:p><w:r><w:drawing><wp:inline><wp:extent cx="457200" cy="228600"/><wp:docPr id="1" name="Logo"/>"#,
            r#"<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic>"#,
            r#"<pic:nvPicPr><pic:cNvPr id="1" name="logo.png"/><pic:cNvPicPr/></pic:nvPicPr>"#,
            r#"<pic:blipFill><a:blip r:embed="rIdLogo"/></pic:blipFill>"#,
            r#"<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="457200" cy="228600"/></a:xfrm></pic:spPr>"#,
            "</pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>",
            "<w:r><w:t> Company</w:t></w:r></w:p></w:hdr>"
        ),
        ns = NAMESPACES,
        r1 = row("Ref", "Date"),
        r2 = row("Owner", "Status"),
    )
}

fn footer_xml() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:ftr {NAMESPACES}><w:p><w:r><w:t>Footer one</w:t></w:r></w:p><w:p><w:r><w:t>Footer two</w:t></w:r></w:p><w:p><w:r><w:t>Footer three</w:t></w:r></w:p></w:ftr>"#
    )
}

/// `sections` sections of `pages` pages each. Only the first section names
/// its header and footer; the others link to the previous section. The
/// bottom margin is shallow, so the footer reaches into it.
fn package(sections: usize, pages: usize) -> Vec<u8> {
    let margins = r#"<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="800" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;
    let references = r#"<w:headerReference w:type="default" r:id="rIdHeader"/><w:footerReference w:type="default" r:id="rIdFooter"/>"#;
    let mut body = String::new();
    for section in 0..sections {
        let sect_pr = if section == 0 {
            format!("<w:sectPr>{references}{margins}</w:sectPr>")
        } else {
            format!("<w:sectPr>{margins}</w:sectPr>")
        };
        for page in 0..pages {
            let text = format!("<w:r><w:t>Section {section} page {page}</w:t></w:r>");
            if page + 1 < pages {
                body.push_str(&format!(
                    r#"<w:p>{text}<w:r><w:br w:type="page"/></w:r></w:p>"#
                ));
            } else if section + 1 < sections {
                body.push_str(&format!("<w:p><w:pPr>{sect_pr}</w:pPr>{text}</w:p>"));
            } else {
                body.push_str(&format!("<w:p>{text}</w:p>{sect_pr}"));
            }
        }
    }
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><w:document {NAMESPACES}><w:body>{body}</w:body></w:document>"#
    );
    let files: Vec<(&str, Vec<u8>)> = vec![
        (
            "[Content_Types].xml",
            concat!(
                r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
                r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
                r#"<Default Extension="xml" ContentType="application/xml"/>"#,
                r#"<Default Extension="png" ContentType="image/png"/>"#,
                r#"<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>"#,
                r#"<Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>"#,
                r#"<Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/>"#,
                "</Types>"
            )
            .into(),
        ),
        (
            "_rels/.rels",
            concat!(
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
                r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>"#,
                "</Relationships>"
            )
            .into(),
        ),
        (
            "word/_rels/document.xml.rels",
            concat!(
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
                r#"<Relationship Id="rIdHeader" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#,
                r#"<Relationship Id="rIdFooter" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/>"#,
                "</Relationships>"
            )
            .into(),
        ),
        (
            "word/_rels/header1.xml.rels",
            concat!(
                r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
                r#"<Relationship Id="rIdLogo" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/logo.png"/>"#,
                "</Relationships>"
            )
            .into(),
        ),
        ("word/document.xml", document.into_bytes()),
        ("word/header1.xml", header_xml().into_bytes()),
        ("word/footer1.xml", footer_xml().into_bytes()),
        ("word/media/logo.png", logo_png()),
    ];
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (path, content) in &files {
            zip.start_file(*path, options).expect("start entry");
            zip.write_all(content).expect("write entry");
        }
        zip.finish().expect("finish package");
    }
    cursor.into_inner()
}

fn load(sections: usize, pages: usize) -> WordDocument {
    futures::executor::block_on(WordDocument::load(
        &package(sections, pages),
        &Options::default(),
    ))
    .expect("load")
}

struct Recording {
    renderer: CanvasRenderer<RecordingPainter>,
    painter: RecordingPainter,
}

/// A renderer on the recording host, as `recording_renderer()` builds one,
/// with the painter it records into.
fn recording(memo: bool) -> Recording {
    let painter = RecordingPainter::new();
    let mut renderer = CanvasRenderer::with_host(
        Box::new(RecordingHost {
            painter: painter.clone(),
        }),
        false,
    )
    .expect("paged recording renderer");
    renderer.hf_story_memo_disabled = !memo;
    Recording { renderer, painter }
}

/// One editor repaint: the stories are installed again from the document,
/// as `canvas_view` does before every render, and the document is rendered.
fn rerender(renderer: &mut CanvasRenderer<RecordingPainter>, word_doc: &WordDocument) {
    renderer.sync_document_header_footer_stories(word_doc);
    let document = Rc::clone(word_doc.document.as_ref().expect("document"));
    renderer
        .render_with_numbering_rc(
            document,
            &Options::default(),
            word_doc.numbering_part.as_ref(),
            word_doc.header_lines.as_deref(),
            word_doc.footer_lines.as_deref(),
            None,
        )
        .expect("render");
}

/// Each drawing call with the painter state it is drawn under; see
/// `header_footer_story_cost_tests::painted`.
fn painted(ops: &[Op]) -> Vec<(Op, String)> {
    let mut settings: BTreeMap<&'static str, Op> = BTreeMap::new();
    let mut transforms: Vec<Op> = Vec::new();
    let mut saved = Vec::new();
    let mut out = Vec::new();
    for op in ops {
        let kind = match op {
            Op::SetFillStyle(_) => "fill",
            Op::SetStrokeStyle(_) => "stroke",
            Op::SetLineWidth(_) => "line_width",
            Op::SetLineDash(_) => "line_dash",
            Op::SetShadowColor(_) => "shadow_color",
            Op::SetShadowBlur(_) => "shadow_blur",
            Op::SetShadowOffsetX(_) => "shadow_x",
            Op::SetShadowOffsetY(_) => "shadow_y",
            Op::SetBlurFilter(_) => "blur",
            Op::SetFont(_) => "font",
            Op::SetTextBaseline(_) => "baseline",
            Op::SetGlobalAlpha(_) => "alpha",
            Op::Save => {
                saved.push((settings.clone(), transforms.clone()));
                continue;
            }
            Op::Restore => {
                (settings, transforms) = saved.pop().unwrap_or_default();
                continue;
            }
            Op::Translate(..) | Op::Scale(..) | Op::Rotate(..) => {
                transforms.push(op.clone());
                continue;
            }
            Op::MeasureText(_) => continue,
            _ => {
                out.push((op.clone(), format!("{settings:?} {transforms:?}")));
                continue;
            }
        };
        settings.insert(kind, op.clone());
    }
    out
}

/// What a render produced that the memo must not change.
#[derive(Debug, PartialEq)]
struct Outcome {
    painted: Vec<(Op, String)>,
    header_boundaries: Vec<u64>,
    footer_boundaries: Vec<Option<u64>>,
    footer_heights: Vec<Option<u64>>,
    body_lines: Vec<(usize, u64, u64)>,
    capture: Vec<String>,
}

fn outcome(recording: &mut Recording, word_doc: &WordDocument, pages: usize) -> (Outcome, usize) {
    recording.painter.clear();
    recording.renderer.hf_story_layout_count = 0;
    rerender(&mut recording.renderer, word_doc);
    let renderer = &recording.renderer;
    let outcome = Outcome {
        painted: painted(&recording.painter.ops()),
        header_boundaries: (0..pages)
            .map(|page| renderer.header_edit_boundary_for_page(page).to_bits())
            .collect(),
        footer_boundaries: (0..pages)
            .map(|page| {
                renderer
                    .footer_edit_boundary_for_page(page)
                    .map(f64::to_bits)
            })
            .collect(),
        footer_heights: renderer
            .footer_content_heights
            .iter()
            .map(|height| height.map(f64::to_bits))
            .collect(),
        body_lines: renderer
            .layout_debug_lines()
            .iter()
            .map(|line| (line.page, line.top.to_bits(), line.height.to_bits()))
            .collect(),
        capture: renderer
            .auxiliary_story_debug_lines()
            .iter()
            .map(|line| format!("{line:?}"))
            .collect(),
    };
    (outcome, renderer.hf_story_layout_count)
}

fn assert_same(with_memo: &Outcome, without: &Outcome, label: &str) {
    assert!(with_memo.painted == without.painted, "{label}: painted ops");
    assert_eq!(
        with_memo.header_boundaries, without.header_boundaries,
        "{label}"
    );
    assert_eq!(
        with_memo.footer_boundaries, without.footer_boundaries,
        "{label}"
    );
    assert_eq!(with_memo.footer_heights, without.footer_heights, "{label}");
    assert_eq!(with_memo.body_lines, without.body_lines, "{label}");
    assert_eq!(with_memo.capture, without.capture, "{label}");
}

const SECTIONS: usize = 50;
const PAGES_PER_SECTION: usize = 2;
const PAGES: usize = SECTIONS * PAGES_PER_SECTION;

#[test]
fn the_fixture_has_a_linked_letterhead_and_a_footer_reaching_the_margin() {
    let word_doc = load(SECTIONS, PAGES_PER_SECTION);
    let sections = word_doc.section_header_footers();
    assert_eq!(sections.len(), SECTIONS);
    let header = sections[SECTIONS - 1]
        .header_body
        .as_ref()
        .expect("the last section links to the first section's header");
    assert!(header
        .elements
        .iter()
        .any(|element| element.as_table().is_some()));
    assert!(
        header
            .elements
            .iter()
            .any(
                |element| element.as_paragraph().is_some_and(|paragraph| paragraph
                    .runs
                    .iter()
                    .any(|run| run
                        .image
                        .as_ref()
                        .is_some_and(|image| image.data_uri.is_some())))
            ),
        "the letterhead holds a data-URI picture"
    );
    assert_eq!(
        sections[SECTIONS - 1].footer.as_ref().map(Vec::len),
        Some(3),
        "footer lines resolve for linked sections"
    );
}

/// Linked sections hand out the one story the part holds, and keep handing
/// it out across repaints until the part is edited.
#[test]
fn linked_sections_share_one_story_until_it_is_edited() {
    let mut word_doc = load(4, 1);
    let first = word_doc.section_header_footers();
    let again = word_doc.section_header_footers();
    let shared = first[0].header_body.as_ref().expect("header");
    for section in first.iter().chain(&again) {
        assert!(Rc::ptr_eq(
            shared,
            section.header_body.as_ref().expect("linked header")
        ));
    }
    // The body-level section names the stories document-wide only when
    // it declares them itself: a one-section document.
    let single = load(1, 1);
    let document_wide = || {
        single
            .header_footer_body(
                HeaderFooterRole::Header,
                crate::word_document::HeaderFooterVariant::Default,
            )
            .expect("document-wide header")
    };
    let section_story = single.section_header_footers()[0]
        .header_body
        .clone()
        .expect("section header");
    assert!(Rc::ptr_eq(&document_wide(), &document_wide()));
    assert!(Rc::ptr_eq(&document_wide(), &section_story));

    // An edit session on the first section edits the part every section
    // links to, as Word does.
    word_doc.hf_edit_section = Some(0);
    word_doc
        .replace_header_paragraph_text(0, "Edited letterhead")
        .expect("edit header");
    let first_text = |body: &Body| -> String {
        body.elements[0]
            .as_paragraph()
            .expect("paragraph")
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect()
    };
    let edited = word_doc.section_header_footers();
    let edited_header = edited[0].header_body.as_ref().expect("header");
    assert!(
        !Rc::ptr_eq(shared, edited_header),
        "an edit hands out a new story"
    );
    assert_eq!(first_text(edited_header), "Edited letterhead");
    let repaint = word_doc.section_header_footers();
    for (section, (after_edit, again)) in edited.iter().zip(&repaint).enumerate() {
        let after_edit = after_edit.header_body.as_ref().expect("header");
        assert!(
            Rc::ptr_eq(after_edit, edited_header),
            "section {section} still shares the edited story"
        );
        assert!(
            Rc::ptr_eq(after_edit, again.header_body.as_ref().expect("header")),
            "section {section} keeps its story across repaints"
        );
    }
}

/// The footer reaches into the bottom margin, so every page measures it in
/// `measure_page_footer` before the body fills the page. With the memo the
/// measurement is reused; nothing the user sees changes.
#[test]
fn undrawn_pages_measure_the_footer_from_the_memo() {
    let word_doc = load(SECTIONS, PAGES_PER_SECTION);
    for visible in [None, Some(49..=52)] {
        let mut memo = recording(true);
        let mut layout = recording(false);
        memo.renderer.set_visible_page_range(visible.clone());
        layout.renderer.set_visible_page_range(visible.clone());
        let (with_memo, memo_layouts) = outcome(&mut memo, &word_doc, PAGES);
        let (without, plain_layouts) = outcome(&mut layout, &word_doc, PAGES);
        assert!(
            with_memo.footer_heights.iter().flatten().count() >= PAGES,
            "measure_page_footer records every page's footer height"
        );
        assert_same(&with_memo, &without, &format!("{visible:?}"));
        assert!(plain_layouts >= 2 * PAGES, "{plain_layouts} layouts");
        if visible.is_some() {
            // Four drawn pages lay out a header and a footer each; every
            // section measures its header and footer once.
            assert!(
                memo_layouts <= 2 * SECTIONS + 16,
                "{memo_layouts} layouts with the memo"
            );
            let (repaint, repaint_layouts) = outcome(&mut memo, &word_doc, PAGES);
            let (plain_repaint, _) = outcome(&mut layout, &word_doc, PAGES);
            assert_same(&repaint, &plain_repaint, "a repaint");
            assert!(repaint_layouts <= 2 * SECTIONS + 16);
        }
    }
}

/// Capturing story lines lays out every page's header, memo or not, and
/// captures the same lines.
#[test]
fn capture_mode_lays_out_every_header() {
    let word_doc = load(8, PAGES_PER_SECTION);
    let pages = 8 * PAGES_PER_SECTION;
    let mut memo = recording(true);
    let mut layout = recording(false);
    for recording in [&mut memo, &mut layout] {
        recording.renderer.set_visible_page_range(Some(1..=1));
        recording.renderer.set_auxiliary_story_capture_enabled(true);
    }
    let (with_memo, memo_layouts) = outcome(&mut memo, &word_doc, pages);
    let (without, _) = outcome(&mut layout, &word_doc, pages);
    let captured_pages = with_memo
        .capture
        .iter()
        .filter(|line| line.contains("story: \"header\""))
        .count();
    assert!(captured_pages >= pages, "{captured_pages} header lines");
    assert!(memo_layouts >= pages, "{memo_layouts} layouts");
    assert!(with_memo == without);
}

/// Header and footer edit modes draw the story's dashed boundary and keep
/// its editable layout on drawn pages; neither depends on the memo.
#[test]
fn edit_modes_paint_and_hit_test_the_same() {
    let word_doc = load(8, PAGES_PER_SECTION);
    let pages = 8 * PAGES_PER_SECTION;
    for mode in [EditingMode::Header, EditingMode::Footer] {
        let mut memo = recording(true);
        let mut layout = recording(false);
        for recording in [&mut memo, &mut layout] {
            recording.renderer.editing_mode = mode;
            recording.renderer.set_visible_page_range(Some(7..=10));
        }
        let (with_memo, _) = outcome(&mut memo, &word_doc, pages);
        let (without, _) = outcome(&mut layout, &word_doc, pages);
        assert!(
            with_memo
                .painted
                .iter()
                .any(|(op, state)| matches!(op, Op::Stroke)
                    && state.contains("SetLineDash([4.0, 4.0])")),
            "{mode:?} strokes the dashed story boundary"
        );
        assert!(with_memo == without, "{mode:?}");
        // Only an all-paragraph story keeps an editable layout; the
        // letterhead holds a table.
        assert!(
            memo.renderer.hf_story_layouts.contains_key(&(6, false)),
            "a drawn page keeps its editable footer layout"
        );
    }
}

/// Text of a story's paragraphs, a field shown by its kind.
fn story_text(body: &Body) -> String {
    fn blocks(elements: &[BodyElement], out: &mut String) {
        for element in elements {
            match element {
                BodyElement::Paragraph(paragraph) => {
                    for run in &paragraph.runs {
                        match &run.field {
                            Some(field) => out.push_str(&format!("{{{:?}}}", field.kind)),
                            None => out.push_str(&run.text),
                        }
                    }
                    out.push('\n');
                }
                BodyElement::Table(table) => {
                    for cell in table.rows.iter().flat_map(|row| &row.cells) {
                        blocks(&cell.blocks, out);
                    }
                }
                BodyElement::Math(_) => {}
            }
        }
    }
    let mut out = String::new();
    blocks(&body.elements, &mut out);
    out
}

/// Every section still links to the first one and shows `expected` in
/// `role`, from one shared part; no part was added.
fn assert_linked_and_shared(word_doc: &WordDocument, header: bool, expected: &str) {
    let role = if header { "header" } else { "footer" };
    let stories = word_doc.section_header_footers();
    let story = |section: &SectionHeaderFooters| {
        if header {
            section.header_body.clone()
        } else {
            section.footer_body.clone()
        }
        .expect("story")
    };
    let shared = story(&stories[0]);
    assert!(
        story_text(&shared).contains(expected),
        "{}",
        story_text(&shared)
    );
    for (index, section) in stories.iter().enumerate() {
        assert!(
            Rc::ptr_eq(&shared, &story(section)),
            "section {index} shares the {role}"
        );
        if index > 0 {
            assert_eq!(
                word_doc.header_footer_linked(index, role, "default"),
                Ok(true),
                "section {index} stays linked"
            );
        }
    }
    let parts = word_doc
        .parts
        .keys()
        .filter(|path| path.starts_with(&format!("word/{role}")))
        .count();
    assert_eq!(parts, 1, "no {role} part is added");

    let reloaded = futures::executor::block_on(WordDocument::load(
        &word_doc.export_docx().expect("export"),
        &Options::default(),
    ))
    .expect("reload");
    for (index, section) in reloaded.section_header_footers().iter().enumerate() {
        assert!(
            story_text(&story(section)).contains(expected),
            "section {index} shows the edit after a reload"
        );
    }
}

/// Without an edit session (headless commands, `insert_page_number`), an
/// edit of the story a linked last section shows goes to the shared part,
/// as Word does; it used to create a blank part for the last section and
/// unlink it, losing the letterhead there.
#[test]
fn sessionless_edits_change_the_shared_linked_part() {
    let mut word_doc = load(4, 1);
    assert_eq!(word_doc.hf_edit_section, None);
    let document_wide = word_doc
        .header_footer_body(
            HeaderFooterRole::Header,
            crate::word_document::HeaderFooterVariant::Default,
        )
        .expect("the last section shows the inherited header");
    assert!(Rc::ptr_eq(
        &document_wide,
        word_doc.section_header_footers()[3]
            .header_body
            .as_ref()
            .expect("header")
    ));

    word_doc
        .replace_header_paragraph_text(0, "Edited letterhead")
        .expect("edit header");
    assert_linked_and_shared(&word_doc, true, "Edited letterhead");
    assert!(
        story_text(
            &word_doc.section_header_footers()[3]
                .header_body
                .clone()
                .expect("header")
        )
        .contains("Ref"),
        "the letterhead's table is kept"
    );

    word_doc
        .insert_page_number(true, "center")
        .expect("insert page number");
    // Like Word's page-number gallery, this replaces the footer's
    // content, in the one part every section shows.
    assert_linked_and_shared(&word_doc, false, "{PageNumber}");
}

/// A watermark and a manuscript grid set on a document whose sections all
/// link to the first section's header are written into that shared header,
/// as Word does, and a reload reads them back from it.
#[test]
fn watermark_and_manuscript_grid_go_into_the_shared_linked_header() {
    let mut word_doc = load(4, 1);
    word_doc
        .set_watermark_text(Some("DRAFT"))
        .expect("set watermark");
    word_doc
        .set_manuscript_grid(Some(jade_docx_reader::document::ManuscriptGrid {
            style: jade_docx_reader::document::ManuscriptStyle::Squares,
            chars: 20,
            lines: 20,
            color: "#00AA00".into(),
        }))
        .expect("set manuscript grid");

    let headers: Vec<&String> = word_doc
        .parts
        .keys()
        .filter(|path| path.starts_with("word/header") && path.ends_with(".xml"))
        .collect();
    assert_eq!(headers, vec!["word/header1.xml"], "no header part is added");
    let shared = &word_doc.parts["word/header1.xml"].content;
    assert!(
        shared.contains("WaterMark"),
        "the shared header holds the watermark"
    );
    assert!(
        shared.contains("Genko:"),
        "the shared header holds the grid"
    );
    for section in 1..4 {
        assert_eq!(
            word_doc.header_footer_linked(section, "header", "default"),
            Ok(true),
            "section {section} stays linked"
        );
    }

    let reloaded = futures::executor::block_on(WordDocument::load(
        &word_doc.export_docx().expect("export"),
        &Options::default(),
    ))
    .expect("reload");
    let document = reloaded.document.as_deref().expect("document");
    assert_eq!(document.watermark_text.as_deref(), Some("DRAFT"));
    assert!(
        document.manuscript_grid.is_some(),
        "the grid survives a reload"
    );
    // The document-wide lines mirror the story the last section shows.
    assert!(
        reloaded.header_lines.is_some(),
        "document-wide header lines load"
    );
}

/// After a session-less edit of a linked story, the document-wide lines
/// (which headless snapshots report) follow the shared part.
#[test]
fn document_wide_lines_follow_a_sessionless_linked_edit() {
    let mut word_doc = load(4, 1);
    word_doc
        .replace_footer_paragraph_text(0, "Edited footer")
        .expect("edit footer");
    let lines = word_doc.footer_lines.as_ref().expect("footer lines");
    let text: String = lines
        .iter()
        .map(|line| line.left.clone().unwrap_or_default())
        .collect();
    assert!(text.contains("Edited footer"), "{text}");
}

fn reload(word_doc: &WordDocument) -> WordDocument {
    futures::executor::block_on(WordDocument::load(
        &word_doc.export_docx().expect("export"),
        &Options::default(),
    ))
    .expect("reload")
}

/// Each section's own default header reference, as the model holds it.
fn own_header_references(word_doc: &WordDocument) -> Vec<Option<String>> {
    word_doc
        .document
        .as_deref()
        .expect("document")
        .sections
        .iter()
        .map(|section| {
            section
                .properties
                .as_ref()
                .and_then(|properties| properties.header_reference.clone())
        })
        .collect()
}

/// The first line of the default header each section shows.
fn shown_headers(word_doc: &WordDocument) -> Vec<String> {
    word_doc
        .section_header_footers()
        .iter()
        .map(|section| {
            section
                .header_body
                .as_deref()
                .and_then(|body| body.elements.first())
                .and_then(BodyElement::as_paragraph)
                .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
                .unwrap_or_default()
        })
        .collect()
}

/// A middle section given a header of its own keeps it through a section
/// break inserted before, in or after it, and with it relinked: the model
/// the editor shows and edits matches what a reload of the saved file
/// reads, and an edit of any section reaches the part that section shows.
/// The unlink used to write the new reference into the section range only,
/// so the break's rebuild of the ranges from the body dropped it while the
/// saved `w:sectPr` kept it.
#[test]
fn an_unlinked_middle_section_keeps_its_header_through_a_section_break() {
    for relink in [false, true] {
        for at in 0..3 {
            let label = format!("relink {relink}, break in section {at}");
            let mut word_doc = load(3, 1);
            word_doc
                .set_header_footer_link(1, "header", "default", false)
                .expect("unlink");
            word_doc.hf_edit_section = Some(1);
            word_doc
                .replace_header_paragraph_text(0, "Own header")
                .expect("edit the unlinked header");
            word_doc.hf_edit_section = None;
            if relink {
                word_doc
                    .set_header_footer_link(1, "header", "default", true)
                    .expect("relink");
            }
            let before = shown_headers(&word_doc);
            word_doc
                .insert_section_break_at(at, 4, crate::word_document::SectionBreakType::NextPage)
                .expect("section break");

            let reloaded = reload(&word_doc);
            assert_eq!(
                own_header_references(&word_doc),
                own_header_references(&reloaded),
                "{label}: model references match the saved file"
            );
            let shown = shown_headers(&word_doc);
            assert_eq!(shown, shown_headers(&reloaded), "{label}: shown headers");
            // The break splits section `at` in two, each showing its header.
            let mut expected = before.clone();
            expected.insert(at, before[at].clone());
            assert_eq!(shown, expected, "{label}");
            if !relink {
                assert!(before[1] == "Own header", "{label}: {before:?}");
            }

            for section in 0..shown.len() {
                let mut edited = word_doc.clone();
                edited.hf_edit_section = Some(section);
                let text = format!("Edit {section}");
                edited
                    .replace_header_paragraph_text(0, &text)
                    .expect("edit");
                edited.hf_edit_section = None;
                let shown = shown_headers(&edited);
                assert_eq!(
                    shown[section], text,
                    "{label}: section {section} shows its edit"
                );
                assert_eq!(
                    shown,
                    shown_headers(&reload(&edited)),
                    "{label}: the edit of section {section} reaches the part it shows"
                );
            }
        }
    }
}

/// Seek snapshots resume the body. A story's paragraphs are numbered past
/// the body's and laid out at body positions, so recording them would hand
/// a seek a resume point inside a header or footer.
#[test]
fn a_story_layout_records_no_seek_snapshot() {
    let word_doc = load(1, 4);
    let mut recording = recording(false);
    rerender(&mut recording.renderer, &word_doc);
    let renderer = &mut recording.renderer;
    let caches = |renderer: &CanvasRenderer<RecordingPainter>| {
        (
            format!("{:?}", renderer.paragraph_seek_cache.borrow()),
            format!("{:?}", renderer.page_start_seek_cache_by_page.borrow()),
            format!("{:?}", renderer.page_start_seek_cache.borrow()),
            format!("{:?}", renderer.page_column_seek_cache.borrow()),
        )
    };
    let before = caches(renderer);
    assert!(
        !renderer.paragraph_seek_cache.borrow().is_empty(),
        "the body recorded snapshots"
    );
    let footer = renderer.footer_body.clone().expect("footer story");
    let base = renderer.auxiliary_story_paragraph_base();
    // At the body's top edge, where a page-start snapshot would be taken.
    let top = renderer.effective_body_top();
    for draw in [false, true] {
        renderer
            .render_header_footer_body_at(&footer, top, base, draw, Some("footer"))
            .expect("story layout");
    }
    assert_eq!(caches(renderer), before, "seek caches are untouched");
}

/// Repaint cost of the linked-section document: the stories are installed
/// again before each render, as the editor does. `cargo test -p
/// jade-docx-editor --lib --release linked_section_repaint_timing --
/// --ignored --nocapture`.
#[test]
#[ignore]
fn linked_section_repaint_timing() {
    let word_doc = load(SECTIONS, PAGES_PER_SECTION);
    let mut recording = recording(true);
    recording.renderer.set_visible_page_range(Some(1..=2));
    rerender(&mut recording.renderer, &word_doc);
    let mut install = Vec::new();
    let mut repaint = Vec::new();
    for _ in 0..15 {
        let started = std::time::Instant::now();
        for _ in 0..10 {
            recording
                .renderer
                .sync_document_header_footer_stories(&word_doc);
        }
        install.push(started.elapsed() / 10);
        let started = std::time::Instant::now();
        rerender(&mut recording.renderer, &word_doc);
        repaint.push(started.elapsed());
    }
    install.sort();
    repaint.sort();
    println!(
        "{SECTIONS} linked sections: median story install {:?}, median repaint {:?}",
        install[install.len() / 2],
        repaint[repaint.len() / 2]
    );
}
