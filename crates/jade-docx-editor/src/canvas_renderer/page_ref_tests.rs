//! `PAGEREF` and `SECTION` results on a recording painter: a PAGEREF shows
//! the number its target's page displays, in that section's format; `\p`
//! keeps Word's saved relative text; a footer's SECTION names the section
//! supplying the page's chrome.

use super::recording_host::recording_renderer;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::{Op, RecordingPainter};
use std::io::Write;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

const GEOMETRY: &str = r#"<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;

/// A `sectPr` sharing the footer `rF`, starting with `break_type`.
fn section_properties(break_type: &str, extra: &str) -> String {
    format!(
        r#"<w:sectPr><w:footerReference w:type="default" r:id="rF"/><w:type w:val="{break_type}"/>{GEOMETRY}{extra}</w:sectPr>"#
    )
}

/// A package of `body` and a footer holding `footer` (a paragraph's inner
/// XML).
fn package(body: &str, footer: &str) -> Vec<u8> {
    use zip::write::SimpleFileOptions;
    let parts = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/></Types>"#.to_string(),
        ),
        (
            "_rels/.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>"#
            ),
        ),
        (
            "word/_rels/document.xml.rels",
            format!(
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rF" Type="{R}/footer" Target="footer1.xml"/></Relationships>"#
            ),
        ),
        (
            "word/document.xml",
            format!(
                r#"<w:document xmlns:w="{W}" xmlns:r="{R}"><w:body>{body}</w:body></w:document>"#
            ),
        ),
        (
            "word/footer1.xml",
            format!(r#"<w:ftr xmlns:w="{W}" xmlns:r="{R}"><w:p>{footer}</w:p></w:ftr>"#),
        ),
    ];
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = SimpleFileOptions::default();
        for (path, content) in parts {
            zip.start_file(path, options).expect("start entry");
            zip.write_all(content.as_bytes()).expect("write entry");
        }
        zip.finish().expect("finish package");
    }
    cursor.into_inner()
}

fn painted_text(renderer: &CanvasRenderer<RecordingPainter>) -> String {
    renderer
        .host
        .painter()
        .inner()
        .ops()
        .iter()
        .filter_map(|op| match op {
            // Paragraph and page-break marks are not field results.
            Op::FillText(text, _, _) if !matches!(text.as_str(), "\u{21b5}" | "\u{21d3}") => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect()
}

/// Renders `data` twice, as a view does once layout knows where a forward
/// reference's target lies, and returns the text the second render painted.
fn render_twice(data: &[u8]) -> String {
    let word_doc =
        futures::executor::block_on(WordDocument::load(data, &Options::default())).expect("load");
    let document = word_doc.document.as_ref().expect("document");
    let mut renderer = recording_renderer();
    renderer.sync_word_document(&word_doc);
    renderer
        .render(document, &Options::default())
        .expect("first render");
    renderer.host.painter().inner().clear();
    renderer
        .render(document, &Options::default())
        .expect("second render");
    painted_text(&renderer)
}

/// Section 1 holds `field` between "[" and "]"; section 2, numbered by
/// `numbering`, holds the bookmark `target`.
fn forward_reference(field: &str, numbering: &str) -> Vec<u8> {
    let body = format!(
        r#"<w:p><w:r><w:t>[</w:t></w:r>{field}<w:r><w:t>]</w:t></w:r></w:p><w:p><w:pPr>{}</w:pPr></w:p><w:p><w:bookmarkStart w:id="0" w:name="target"/><w:r><w:t>T</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p>{}"#,
        section_properties("nextPage", ""),
        section_properties("nextPage", numbering),
    );
    package(
        &body,
        r#"<w:r><w:t>F</w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple>"#,
    )
}

/// Word shows the target page's displayed number: section 2 restarts at 1
/// in lower-case roman numerals, so physical page 2 shows "i".
#[test]
fn a_pageref_shows_the_roman_number_its_targets_section_displays() {
    let text = render_twice(&forward_reference(
        r#"<w:fldSimple w:instr=" PAGEREF target \h "><w:r><w:t>9</w:t></w:r></w:fldSimple>"#,
        r#"<w:pgNumType w:fmt="lowerRoman" w:start="1"/>"#,
    ));
    assert!(text.contains("[i]"), "{text:?}");
}

/// A restart applies without a format change.
#[test]
fn a_pageref_follows_its_targets_section_restart() {
    let text = render_twice(&forward_reference(
        r#"<w:fldSimple w:instr=" PAGEREF target "><w:r><w:t>9</w:t></w:r></w:fldSimple>"#,
        r#"<w:pgNumType w:start="5"/>"#,
    ));
    assert!(text.contains("[5]"), "{text:?}");
}

/// A section without `w:fmt` numbers in decimal: it does not take the
/// roman format of the section after it. Page 1's footer PAGE shows "1",
/// page 2's "i", and the PAGEREF on page 1 to page 2 shows "i".
#[test]
fn a_decimal_section_before_a_roman_one_keeps_decimal_numbers() {
    let text = render_twice(&forward_reference(
        r#"<w:fldSimple w:instr=" PAGEREF target "><w:r><w:t>9</w:t></w:r></w:fldSimple>"#,
        r#"<w:pgNumType w:fmt="lowerRoman" w:start="1"/>"#,
    ));
    assert_eq!(text, "[i]F1TFi");
}

/// A PAGEREF from a roman section back to a decimal one shows the decimal
/// number of the target's page.
#[test]
fn a_pageref_into_a_decimal_section_before_a_roman_one_is_decimal() {
    let body = format!(
        r#"<w:p><w:r><w:t>D</w:t></w:r></w:p><w:p><w:r><w:br w:type="page"/></w:r></w:p><w:p><w:bookmarkStart w:id="0" w:name="target"/><w:r><w:t>T</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p><w:p><w:pPr>{}</w:pPr></w:p><w:p><w:r><w:t>[</w:t></w:r><w:fldSimple w:instr=" PAGEREF target "><w:r><w:t>9</w:t></w:r></w:fldSimple><w:r><w:t>]</w:t></w:r></w:p>{}"#,
        section_properties("nextPage", ""),
        section_properties(
            "nextPage",
            r#"<w:pgNumType w:fmt="lowerRoman" w:start="1"/>"#
        ),
    );
    let text = render_twice(&package(
        &body,
        r#"<w:r><w:t>F</w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple>"#,
    ));
    assert_eq!(text, "DF1TF2[2]Fi");
}

/// `\p` prints relative text, which Jade does not generate: the saved
/// result stays.
#[test]
fn a_relative_pageref_paints_its_saved_result() {
    let text = render_twice(&forward_reference(
        r#"<w:fldSimple w:instr=" PAGEREF target \p "><w:r><w:t>on page 2</w:t></w:r></w:fldSimple>"#,
        r#"<w:pgNumType w:fmt="lowerRoman" w:start="1"/>"#,
    ));
    assert!(text.contains("[on page 2]"), "{text:?}");
}

#[test]
fn the_parser_keeps_pagerefs_relative_position_switch() {
    let word_doc = futures::executor::block_on(WordDocument::load(
        &forward_reference(
            r#"<w:fldSimple w:instr=" PAGEREF target \h \p "><w:r><w:t>above</w:t></w:r></w:fldSimple>"#,
            "",
        ),
        &Options::default(),
    ))
    .expect("load");
    let kind = word_doc
        .paragraph(0)
        .expect("paragraph")
        .runs
        .iter()
        .find_map(|run| run.field.as_ref())
        .map(|field| field.kind.clone());
    assert_eq!(
        kind,
        Some(RunFieldKind::PageRef {
            name: "target".into(),
            hyperlink: true,
            relative_position: true,
        })
    );
}

/// A footer's SECTION on a page shared across a continuous break names the
/// section at the page's top, whose footer the page shows; later pages
/// name the second section.
#[test]
fn a_footers_section_field_follows_the_pages_chrome_section() {
    let a: String = (0..3)
        .map(|index| format!("<w:p><w:r><w:t>a{index}</w:t></w:r></w:p>"))
        .collect();
    let b: String = (0..80)
        .map(|index| format!("<w:p><w:r><w:t>b{index}</w:t></w:r></w:p>"))
        .collect();
    let body = format!(
        "{a}<w:p><w:pPr>{}</w:pPr></w:p>{b}{}",
        section_properties("nextPage", ""),
        section_properties("continuous", ""),
    );
    let data = package(
        &body,
        r#"<w:r><w:t>S</w:t></w:r><w:fldSimple w:instr=" SECTION "><w:r><w:t>0</w:t></w:r></w:fldSimple><w:r><w:t>;</w:t></w:r>"#,
    );
    let word_doc =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let document = word_doc.document.as_ref().expect("document");
    let mut renderer = recording_renderer();
    renderer.sync_word_document(&word_doc);
    renderer
        .render(document, &Options::default())
        .expect("render");
    let text = painted_text(&renderer);
    assert!(text.contains("a2b0"), "B starts on page 1: {text:?}");
    let sections: Vec<&str> = text
        .split('S')
        .skip(1)
        .filter_map(|rest| rest.split(';').next())
        .collect();
    assert_eq!(sections, ["1", "2", "2"], "{text:?}");
}

/// The keep-next measurer resolves a forward PAGEREF as the body paints
/// it: from the previous pass's page sections. (A group at the top of the
/// page is not measured, hence the leading paragraph.)
#[test]
fn a_keep_next_chains_forward_pageref_measures_as_it_paints() {
    let field = r#"<w:fldSimple w:instr=" PAGEREF target "><w:r><w:t>9</w:t></w:r></w:fldSimple>"#;
    let body = format!(
        r#"<w:p><w:r><w:t>lead</w:t></w:r></w:p><w:p><w:pPr><w:keepNext/></w:pPr><w:r><w:t>[</w:t></w:r>{field}<w:r><w:t>]</w:t></w:r></w:p><w:p><w:r><w:t>next</w:t></w:r></w:p><w:p><w:pPr>{}</w:pPr></w:p><w:p><w:bookmarkStart w:id="0" w:name="target"/><w:r><w:t>T</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p>{}"#,
        section_properties("nextPage", ""),
        section_properties(
            "nextPage",
            r#"<w:pgNumType w:fmt="lowerRoman" w:start="1"/>"#
        ),
    );
    let data = package(&body, "<w:r><w:t>F</w:t></w:r>");
    let word_doc =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let document = word_doc.document.as_ref().expect("document");
    let mut renderer = recording_renderer();
    renderer.sync_word_document(&word_doc);
    for _ in 0..2 {
        renderer
            .render(document, &Options::default())
            .expect("render");
    }
    assert!(painted_text(&renderer).contains("[i]"));
    let measurer = renderer
        .keep_next_measurer
        .as_ref()
        .expect("the keepNext chain was measured");
    assert_eq!(
        measurer.page_sections_before_pass,
        renderer.page_sections_before_pass
    );
    assert_eq!(measurer.page_reference_number(2), "i");
}

/// A session over a loaded document and the renderer that laid it out,
/// as the browser's detached session reads its layout facts.
struct LaidOutSession {
    document: WordDocument,
    renderer: CanvasRenderer<RecordingPainter>,
    selection: Option<jade_selection::SelectionRange>,
    executor: crate::commands::CommandExecutor,
    /// Whether the session reports the renderer's page labels.
    labels: bool,
}

impl crate::word::WordSession for LaidOutSession {
    fn word_document(&self) -> &WordDocument {
        &self.document
    }
    fn selection_range(&self) -> Option<jade_selection::SelectionRange> {
        self.selection
    }
    fn set_selection_range(&mut self, range: Option<jade_selection::SelectionRange>) {
        self.selection = range;
    }
    fn apply_command(
        &mut self,
        _command: jade_docx_headless_api::HeadlessCommand,
        _keep: bool,
    ) -> Result<(), String> {
        Err("not used".into())
    }
    fn apply_commands(
        &mut self,
        _commands: Vec<jade_docx_headless_api::HeadlessCommand>,
        _keep: bool,
    ) -> Result<(), String> {
        Err("not used".into())
    }
    fn execute_command(
        &mut self,
        command: Box<dyn crate::commands::Command>,
        _keep: bool,
    ) -> Result<(), String> {
        let mut context =
            crate::commands::CommandContext::new(&mut self.document, &mut self.selection);
        self.executor
            .execute(command, &mut context)
            .map(|_| ())
            .map_err(|error| format!("{error:?}"))
    }
    fn cell_id_for_paragraph(&self, _paragraph: usize) -> Option<usize> {
        None
    }
    fn cell_layout(&self, _cell_id: usize) -> Option<&crate::word::CellLayout> {
        None
    }
    fn cell_layouts(&self) -> &[crate::word::CellLayout] {
        &[]
    }
    fn table_order_for(&self, _table_id: usize) -> Option<usize> {
        None
    }
    fn hyperlink_regions(&self) -> Vec<HyperlinkRegion> {
        Vec::new()
    }
    fn select_cells(&mut self, _anchor: usize, _focus: usize) -> Result<(), String> {
        Err("not used".into())
    }
    // The recording host always reports one page; count what was laid out.
    fn page_count(&self) -> Option<usize> {
        Some(self.renderer.displayed_page_labels().len())
    }
    fn paragraph_page(&self, paragraph: usize) -> Option<usize> {
        self.renderer.cached_paragraph_start_page(paragraph)
    }
    fn page_label(&self, page: usize) -> Option<crate::fields::PageLabel> {
        if !self.labels {
            return None;
        }
        self.renderer
            .displayed_page_labels()
            .get(page.checked_sub(1)?)
            .cloned()
    }
}

/// Updating a table of contents' page numbers writes the number the
/// target page displays — "i" in a lower-roman section restarting at 1,
/// not its physical page 2 — and leaves a `\p` entry's saved text.
#[test]
fn updating_toc_page_numbers_writes_the_displayed_number() {
    use crate::word::references::toc::WordToc;
    use crate::word::WordSession;
    let body = format!(
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Chapter</w:t></w:r><w:r><w:tab/></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGEREF target \h </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>9</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p><w:p><w:r><w:t xml:space="preserve">Rel </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGEREF target \p </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>below</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p><w:p><w:pPr>{}</w:pPr></w:p><w:p><w:bookmarkStart w:id="0" w:name="target"/><w:r><w:t>T</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p>{}"#,
        section_properties("nextPage", ""),
        section_properties(
            "nextPage",
            r#"<w:pgNumType w:fmt="lowerRoman" w:start="1"/>"#
        ),
    );
    let data = package(&body, "<w:r><w:t>F</w:t></w:r>");
    let document =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let mut renderer = recording_renderer();
    renderer.sync_word_document(&document);
    let model = document.document.clone().expect("document");
    for _ in 0..2 {
        renderer
            .render(&model, &Options::default())
            .expect("render");
    }
    let mut session = LaidOutSession {
        document,
        renderer,
        selection: Some(jade_selection::SelectionRange::collapsed(
            jade_selection::TextPosition::new(0, 0),
        )),
        executor: crate::commands::CommandExecutor::new(),
        labels: true,
    };
    assert_eq!(session.paragraph_page(3), Some(2));
    session.update_toc(true).expect("update the page numbers");

    let text = |document: &WordDocument, index: usize| -> String {
        document
            .paragraph(index)
            .expect("paragraph")
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect()
    };
    assert_eq!(text(&session.document, 0), "Chapter\ti");
    assert_eq!(text(&session.document, 1), "Rel below");
    let exported = session.document.export_docx().expect("export");
    let reloaded = futures::executor::block_on(WordDocument::load(&exported, &Options::default()))
        .expect("reload");
    assert_eq!(text(&reloaded, 0), "Chapter\ti");
    assert_eq!(text(&reloaded, 1), "Rel below");
}

/// Loads `body` (with a footer "F"), lays it out twice, and returns a
/// session over it with the caret at the start.
fn laid_out_session(body: &str) -> LaidOutSession {
    let data = package(body, "<w:r><w:t>F</w:t></w:r>");
    let document =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let mut renderer = recording_renderer();
    renderer.sync_word_document(&document);
    let model = document.document.clone().expect("document");
    for _ in 0..2 {
        renderer
            .render(&model, &Options::default())
            .expect("render");
    }
    LaidOutSession {
        document,
        renderer,
        selection: Some(jade_selection::SelectionRange::collapsed(
            jade_selection::TextPosition::new(0, 0),
        )),
        executor: crate::commands::CommandExecutor::new(),
        labels: true,
    }
}

/// The text of each paragraph after exporting and reloading: its runs'
/// text, and a field's saved result where the field keeps it apart from
/// the run text (PAGE).
fn exported_paragraphs(document: &WordDocument) -> Vec<String> {
    let exported = document.export_docx().expect("export");
    let reloaded = futures::executor::block_on(WordDocument::load(&exported, &Options::default()))
        .expect("reload");
    (0..reloaded.paragraph_count())
        .map(|index| {
            reloaded
                .paragraph(index)
                .expect("paragraph")
                .runs
                .iter()
                .map(|run| match run.field.as_ref() {
                    Some(field) if run.text.is_empty() => {
                        field.cached_text.clone().unwrap_or_default()
                    }
                    _ => run.text.clone(),
                })
                .collect()
        })
        .collect()
}

fn complex_field(instruction: &str, result: &str) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {instruction} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>{result}</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
    )
}

const ROMAN_RESTART: &str = r#"<w:pgNumType w:fmt="lowerRoman" w:start="1"/>"#;

/// Updates every field of a two-page document: section 1 (numbered by
/// `first`) holds a PAGEREF to the start of section 2 (numbered by
/// `second`), which holds a PAGE and a `PAGE \* Arabic`. Returns the
/// exported text of the two paragraphs.
fn updated_page_fields(first: &str, second: &str) -> (String, String) {
    use crate::word::fields::{FieldHost, WordFields};
    let body = format!(
        r#"<w:p><w:r><w:t>[</w:t></w:r>{}<w:r><w:t>]</w:t></w:r></w:p><w:p><w:pPr>{}</w:pPr></w:p><w:p><w:bookmarkStart w:id="0" w:name="target"/><w:r><w:t>T</w:t></w:r><w:bookmarkEnd w:id="0"/><w:r><w:t>(</w:t></w:r>{}<w:r><w:t>|</w:t></w:r>{}<w:r><w:t>)</w:t></w:r></w:p>{}"#,
        complex_field("PAGEREF target \\h", "9"),
        section_properties("nextPage", first),
        complex_field("PAGE", "9"),
        complex_field("PAGE \\* Arabic", "9"),
        section_properties("nextPage", second),
    );
    let mut session = laid_out_session(&body);
    session
        .update_fields(
            crate::fields::FieldScope::Document,
            &FieldHost::default(),
            HashMap::new(),
        )
        .expect("update the fields");
    let paragraphs = exported_paragraphs(&session.document);
    (paragraphs[0].clone(), paragraphs[2].clone())
}

/// F9 writes the number the layout displays: a PAGEREF into, and a PAGE
/// in, a lower-roman section restarting at 1 give "i", where the physical
/// page is 2. A PAGE with its own numeric switch formats the PAGE value.
#[test]
fn updating_fields_writes_the_displayed_page_number() {
    assert_eq!(
        updated_page_fields("", ROMAN_RESTART),
        ("[i]".to_string(), "T(i|1)".to_string())
    );
}

/// A lower-roman section that continues from a first section starting at
/// 5 shows "vi" on physical page 2, as the layout numbers it.
#[test]
fn updating_fields_follows_the_layouts_continued_numbering() {
    assert_eq!(
        updated_page_fields(
            r#"<w:pgNumType w:start="5"/>"#,
            r#"<w:pgNumType w:fmt="lowerRoman"/>"#
        ),
        ("[vi]".to_string(), "T(vi|6)".to_string())
    );
}

/// Rebuilding a table of contents writes each entry's displayed page
/// number where the session has a layout.
#[test]
fn rebuilding_a_toc_writes_the_displayed_page_number() {
    use crate::word::references::toc::WordToc;
    let body = format!(
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>stale</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p><w:p><w:pPr>{}</w:pPr></w:p><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Chapter</w:t></w:r></w:p><w:p><w:r><w:t>body</w:t></w:r></w:p>{}"#,
        section_properties("nextPage", ""),
        section_properties("nextPage", ROMAN_RESTART),
    );
    let mut session = laid_out_session(&body);
    session.update_toc(false).expect("rebuild the table");
    let paragraphs = exported_paragraphs(&session.document);
    let entry = paragraphs
        .iter()
        .find(|text| text.starts_with("Chapter"))
        .expect("an entry for the heading");
    assert!(entry.ends_with("\ti"), "{paragraphs:?}");
}

/// Without page labels a rebuild keeps the generated placeholders.
#[test]
fn rebuilding_a_toc_without_labels_keeps_its_placeholders() {
    use crate::word::references::toc::WordToc;
    let body = format!(
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>stale</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p><w:p><w:pPr>{}</w:pPr></w:p><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Chapter</w:t></w:r></w:p>{}"#,
        section_properties("nextPage", ""),
        section_properties("nextPage", ROMAN_RESTART),
    );
    let mut session = laid_out_session(&body);
    session.labels = false;
    session.update_toc(false).expect("rebuild the table");
    let paragraphs = exported_paragraphs(&session.document);
    let entry = paragraphs
        .iter()
        .find(|text| text.starts_with("Chapter"))
        .expect("an entry for the heading");
    assert!(entry.ends_with("\t1"), "{paragraphs:?}");
}

/// Section A, then a continuous section B in lower-roman numerals from 7
/// that starts on page 2 (shared with A). A TOC entry and a PAGEREF point
/// at B's first paragraph, which holds a PAGE field. After a full layout,
/// a pixel-only rerender of page 1 stops its walk on page 2 while still in
/// A.
fn session_after_rerender() -> LaidOutSession {
    let filler: String = (0..60)
        .map(|index| format!("<w:p><w:r><w:t>a{index}</w:t></w:r></w:p>"))
        .collect();
    let tail: String = (0..60)
        .map(|index| format!("<w:p><w:r><w:t>b{index}</w:t></w:r></w:p>"))
        .collect();
    let body = format!(
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Chapter</w:t></w:r><w:r><w:tab/></w:r>{}<w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>{filler}<w:p><w:pPr>{}</w:pPr></w:p><w:p><w:bookmarkStart w:id="0" w:name="target"/><w:r><w:t>T(</w:t></w:r><w:bookmarkEnd w:id="0"/>{}<w:r><w:t>)</w:t></w:r></w:p>{tail}{}"#,
        complex_field("PAGEREF target \\h", "9"),
        section_properties("nextPage", ""),
        complex_field("PAGE", "9"),
        section_properties(
            "continuous",
            r#"<w:pgNumType w:fmt="lowerRoman" w:start="7"/>"#
        ),
    );
    let mut session = laid_out_session(&body);
    let labels = |session: &LaidOutSession| -> Vec<String> {
        session
            .renderer
            .displayed_page_labels()
            .into_iter()
            .map(|label| label.text)
            .collect()
    };
    assert_eq!(labels(&session), ["1", "vii", "viii", "ix"]);
    let model = session.document.document.clone().expect("document");
    session.renderer.set_visible_page_range(Some(1..=1));
    session.renderer.set_layout_page_limit(Some(1));
    session.renderer.prepare_rerender_for_page(0);
    session.renderer.freeze_selection_for_pixel_rerender();
    session
        .renderer
        .render(&model, &Options::default())
        .expect("pixel rerender");
    assert_eq!(labels(&session), ["1", "vii", "viii", "ix"]);
    session
}

/// A pixel-only rerender leaves the labels the last full layout recorded,
/// so updating a TOC's page numbers still writes "vii", not "2".
#[test]
fn a_toc_update_after_a_pixel_rerender_writes_the_laid_out_label() {
    use crate::word::references::toc::WordToc;
    let mut session = session_after_rerender();
    session.update_toc(true).expect("update the page numbers");
    assert_eq!(exported_paragraphs(&session.document)[0], "Chapter\tvii");
}

/// The same holds for F9: the PAGE field on the shared page shows "vii".
#[test]
fn f9_after_a_pixel_rerender_writes_the_laid_out_label() {
    use crate::word::fields::{FieldHost, WordFields};
    let mut session = session_after_rerender();
    session
        .update_fields(
            crate::fields::FieldScope::Document,
            &FieldHost::default(),
            HashMap::new(),
        )
        .expect("update the fields");
    let paragraphs = exported_paragraphs(&session.document);
    assert!(
        paragraphs.iter().any(|text| text == "T(vii)"),
        "{paragraphs:?}"
    );
}

/// Three sections over about eight pages: A in decimal, B in lower-roman
/// from 7, C from 50. A TOC entry in A points at C's first paragraph,
/// which holds a PAGE field. After a full layout, sections B and C switch
/// to upper-case letters from 3, and one pass lays out only the first
/// three pages. Labels past the page that pass stopped on come from the
/// last full layout's page owners, numbered with the new formats.
fn session_after_numbering_edit() -> LaidOutSession {
    let paragraphs = |prefix: &str, count: usize| -> String {
        (0..count)
            .map(|index| format!("<w:p><w:r><w:t>{prefix}{index}</w:t></w:r></w:p>"))
            .collect()
    };
    let body = format!(
        r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \o "1-3" \h </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Chapter</w:t></w:r><w:r><w:tab/></w:r>{}<w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>{}<w:p><w:pPr>{}</w:pPr></w:p>{}<w:p><w:pPr>{}</w:pPr></w:p><w:p><w:bookmarkStart w:id="0" w:name="target"/><w:r><w:t>T(</w:t></w:r><w:bookmarkEnd w:id="0"/>{}<w:r><w:t>)</w:t></w:r></w:p>{}{}"#,
        complex_field("PAGEREF target \\h", "9"),
        paragraphs("a", 60),
        section_properties("nextPage", ""),
        paragraphs("b", 120),
        section_properties(
            "nextPage",
            r#"<w:pgNumType w:fmt="lowerRoman" w:start="7"/>"#
        ),
        complex_field("PAGE", "9"),
        paragraphs("c", 70),
        section_properties("nextPage", r#"<w:pgNumType w:start="50"/>"#),
    );
    let mut session = laid_out_session(&body);
    let labels = |session: &LaidOutSession| -> Vec<String> {
        session
            .renderer
            .displayed_page_labels()
            .into_iter()
            .map(|label| label.text)
            .collect()
    };
    let before = labels(&session);
    let c_first = before
        .iter()
        .position(|label| label == "50")
        .expect("C starts at 50");
    for section in [1, 2] {
        session
            .document
            .set_page_number_format("upperLetter", Some(3), Some(section))
            .expect("renumber the section");
    }
    let model = session.document.document.clone().expect("document");
    session.renderer.set_layout_page_limit(Some(3));
    session
        .renderer
        .render(&model, &Options::default())
        .expect("a pass over the first three pages");
    session.renderer.set_layout_page_limit(None);
    assert!(!session.renderer.last_layout_complete);
    let after = labels(&session);
    assert_eq!(after.len(), before.len(), "{after:?}");
    assert_eq!(after[c_first], "C", "{after:?}");
    assert!(
        after
            .iter()
            .all(|label| !label.chars().any(|c| c.is_ascii_digit())
                || label == "1"
                || label == "2"),
        "only A's pages are decimal: {after:?}"
    );
    session
}

/// After a numbering-only edit and an incomplete pass, updating the TOC's
/// page numbers writes C's new first number, not the old "50".
#[test]
fn a_toc_update_after_a_numbering_edit_and_a_limited_pass_writes_the_new_label() {
    use crate::word::references::toc::WordToc;
    let mut session = session_after_numbering_edit();
    session.update_toc(true).expect("update the page numbers");
    assert_eq!(exported_paragraphs(&session.document)[0], "Chapter\tC");
}

/// The same for F9: the PAGE field on C's first page shows "C".
#[test]
fn f9_after_a_numbering_edit_and_a_limited_pass_writes_the_new_label() {
    use crate::word::fields::{FieldHost, WordFields};
    let mut session = session_after_numbering_edit();
    session
        .update_fields(
            crate::fields::FieldScope::Document,
            &FieldHost::default(),
            HashMap::new(),
        )
        .expect("update the fields");
    let paragraphs = exported_paragraphs(&session.document);
    assert!(
        paragraphs.iter().any(|text| text == "T(C)"),
        "{paragraphs:?}"
    );
}

/// Print ranges and the TOC read the same labels after a pixel rerender.
#[test]
fn print_range_labels_agree_with_displayed_labels_after_a_rerender() {
    let session = session_after_rerender();
    let displayed: Vec<usize> = session
        .renderer
        .displayed_page_labels()
        .into_iter()
        .map(|label| label.value)
        .collect();
    let printed: Vec<usize> = session
        .renderer
        .labels_from_recorded_owners(displayed.len())
        .into_iter()
        .map(|(_, value)| value)
        .collect();
    assert_eq!(printed, displayed);
    assert_eq!(displayed, [1, 7, 8, 9]);
}
