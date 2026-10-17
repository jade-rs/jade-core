use super::{DocxXmlDocument, XmlElement, XmlNode, XmlNodeId};
use crate::document::{
    BodyElement, Document, Paragraph, Run, RunFieldKind, Table, TableCell, TableRow,
};
use crate::parser::DocumentParser;

#[derive(Debug)]
pub enum DocxMappingError {
    MissingBody,
}

pub fn attach_document_xml_ids(
    document: &mut Document,
    xml: &DocxXmlDocument,
) -> Result<(), DocxMappingError> {
    let Some(body) = document.body.as_mut() else {
        return Ok(());
    };
    let xml_body = find_body(xml).ok_or(DocxMappingError::MissingBody)?;
    map_body_elements(body.elements.as_mut_slice(), xml_body, xml);
    Ok(())
}

pub fn attach_text_box_xml_ids(document: &mut Document, xml: &DocxXmlDocument) {
    let Some(body) = document.body.as_mut() else {
        return;
    };
    map_text_box_elements(body.elements.as_mut_slice(), xml);
}

/// Links the blocks of a story other than the body — a footnote, an
/// endnote — to the block children of its XML element, as
/// [`attach_document_xml_ids`] does for the body.
pub fn attach_block_xml_ids(
    elements: &mut [BodyElement],
    xml_parent: &XmlElement,
    doc: &DocxXmlDocument,
) {
    map_body_elements(elements, xml_parent, doc);
}

/// Unlinks blocks from the XML tree [`attach_block_xml_ids`] linked them to.
pub fn clear_block_xml_ids(elements: &mut [BodyElement]) {
    clear_block_ids(elements);
}

fn map_body_elements(elements: &mut [BodyElement], xml_parent: &XmlElement, doc: &DocxXmlDocument) {
    let mut xml_blocks = Vec::new();
    collect_transparently_wrapped_children(
        &xml_parent.children,
        &["w:p", "w:tbl"],
        &mut xml_blocks,
    );
    let mut iter = xml_blocks.into_iter().peekable();
    let mut math_wrapper_with_runs = None;

    for element in elements {
        match element {
            BodyElement::Paragraph(paragraph) => {
                let xml_paragraph = math_wrapper_with_runs
                    .take()
                    .or_else(|| find_next_with_name(&mut iter, "w:p"));
                if let Some(xml_paragraph) = xml_paragraph {
                    map_paragraph(paragraph.as_mut(), xml_paragraph, doc);
                } else {
                    paragraph.as_mut().node_id = None;
                }
            }
            BodyElement::Table(table) => {
                math_wrapper_with_runs = None;
                if let Some(xml_table) = find_next_with_name(&mut iter, "w:tbl") {
                    map_table(table.as_mut(), xml_table, doc);
                } else {
                    table.as_mut().node_id = None;
                }
            }
            BodyElement::Math(_) => {
                math_wrapper_with_runs = None;
                let is_math_wrapper = iter.peek().is_some_and(|paragraph| {
                    paragraph.name == "w:p"
                        && element_children(paragraph).any(|child| child.name == "m:oMathPara")
                });
                if is_math_wrapper {
                    let xml_paragraph = iter.next().expect("peeked math wrapper");
                    let mut slots = Vec::new();
                    collect_run_slots(&xml_paragraph.children, None, doc, &mut slots);
                    if !slots.is_empty() {
                        // The parser emits both a Math block and an editable
                        // Paragraph when a wrapper also contains Word runs.
                        math_wrapper_with_runs = Some(xml_paragraph);
                    }
                }
            }
        }
    }
}

fn map_paragraph(paragraph: &mut Paragraph, xml_paragraph: &XmlElement, doc: &DocxXmlDocument) {
    paragraph.node_id = Some(xml_paragraph.id);

    for run in &mut paragraph.runs {
        run.node_id = None;
        run.text_node_id = None;
        run.revision_node_id = None;
        if let Some(field) = run.field.as_mut() {
            field.source_field_id = None;
        }
    }

    let mut slots = Vec::new();
    collect_run_slots(&xml_paragraph.children, None, doc, &mut slots);
    let mut slots = slots.into_iter();

    for run in &mut paragraph.runs {
        // A bare slot whose content the parser did not turn into a run (a
        // `w:fldSimple` nested in another field's instruction) owns no model
        // run. Skip it rather than leave this run without its `w:r`.
        let Some(slot) = slots.find(|slot| match slot {
            RunSlot::Element(..) => true,
            RunSlot::Bare(bare) => bare.is_modeled_by(run),
        }) else {
            break;
        };
        let RunSlot::Element(xml_run, wrapper_id) = slot else {
            // Bare content has no `w:r`. Empty fields still name their
            // source container so the writer can reuse it exactly once.
            if let RunSlot::Bare(BareRun::Field { container, .. }) = slot {
                if let Some(field) = run.field.as_mut() {
                    field.source_field_id = Some(container);
                }
            }
            continue;
        };
        map_run(run, xml_run);
        if let Some(field) = run.field.as_mut() {
            let mut ancestor = doc.parent_id(xml_run.id);
            field.source_field_id = None;
            while let Some(id) = ancestor {
                let Some(element) = doc.element(id) else {
                    break;
                };
                if element.name == "w:fldSimple" {
                    field.source_field_id = Some(id);
                    break;
                }
                if element.name == "w:p" {
                    break;
                }
                ancestor = doc.parent_id(id);
            }
        }
        run.revision_node_id = wrapper_id;
    }

    // Ensure revision_node_id is valid; if wrapper no longer exists, clear it.
    for run in &mut paragraph.runs {
        if let Some(wrapper_id) = run.revision_node_id {
            if doc.element(wrapper_id).is_none() {
                run.revision_node_id = None;
            }
        }
    }
}

fn map_run(run: &mut Run, xml_run: &XmlElement) {
    run.node_id = Some(xml_run.id);
    run.text_node_id = element_children(xml_run)
        .find(|child| child.name == "w:t" || child.name == "w:delText")
        .map(|child| child.id);
}

/// One position in a paragraph that the parser turns into one model run.
/// Model runs pair with slots in order, so every slot must own exactly one
/// run, and content that becomes a run without a `w:r` needs a slot too:
/// otherwise every later run takes its left neighbour's `w:r`.
enum RunSlot<'a> {
    /// A `w:r` element, with the `w:ins`/`w:del` that wraps it.
    Element(&'a XmlElement, Option<XmlNodeId>),
    /// Content the parser models as a run of its own with no `w:r`.
    Bare(BareRun),
}

enum BareRun {
    /// An inline `m:oMath` outside any `w:r`.
    Math,
    /// A `w:fldSimple` without result runs: the parser keeps the field as a
    /// zero-width run.
    Field {
        container: XmlNodeId,
        kind: Box<RunFieldKind>,
    },
    /// A `w:object` outside any `w:r`.
    Object,
}

impl BareRun {
    /// Whether `run` has the shape the parser builds for this content. The
    /// collector cannot see everything the parser decides (a field nested in
    /// another field's instruction makes no run), so pairing checks the run
    /// instead of trusting the slot.
    fn is_modeled_by(&self, run: &Run) -> bool {
        run.text.is_empty()
            && match self {
                BareRun::Math => run.math.is_some(),
                BareRun::Field { kind, .. } => {
                    run.math.is_none()
                        && run.field.as_ref().is_some_and(|field| {
                            field.kind == **kind && field.cached_text.is_none()
                        })
                }
                BareRun::Object => run.image.is_some(),
            }
    }
}

fn collect_run_slots<'a>(
    nodes: &'a [XmlNode],
    wrapper: Option<XmlNodeId>,
    doc: &DocxXmlDocument,
    out: &mut Vec<RunSlot<'a>>,
) {
    for node in nodes {
        let XmlNode::Element(element) = node else {
            continue;
        };
        // The parser models no `w:r`, `m:oMath` or `w:object`
        // written in empty-element form (`<w:r/>` has no text and no
        // properties). Pairing must skip them, or every later run takes its
        // left neighbour's element and the next edit re-emits that
        // neighbour's formatting (jlsi/jade#402). Removing a comment reference
        // run leaves exactly `<w:r/>` behind.
        if doc.is_self_closing(element.id) && element.name != "w:fldSimple" {
            continue;
        }
        match element.name.as_str() {
            "w:r" if is_ruby_run(element) => {
                for ruby in element_children(element).filter(|child| child.name == "w:ruby") {
                    for base in element_children(ruby).filter(|child| child.name == "w:rubyBase") {
                        collect_run_slots(&base.children, wrapper, doc, out);
                    }
                }
            }
            "w:r" => out.push(RunSlot::Element(element, wrapper)),
            "w:ins" | "w:del" => collect_run_slots(&element.children, Some(element.id), doc, out),
            "m:oMath" => out.push(RunSlot::Bare(BareRun::Math)),
            "w:object" => out.push(RunSlot::Bare(BareRun::Object)),
            name if is_transparent_run_container(name) => {
                let start = out.len();
                collect_run_slots(&element.children, wrapper, doc, out);
                if name == "w:fldSimple" && out.len() == start {
                    if let Some(kind) = simple_field_kind(element) {
                        out.push(RunSlot::Bare(BareRun::Field {
                            container: element.id,
                            kind: Box::new(kind),
                        }));
                    }
                }
            }
            _ => {}
        }
    }
}

fn simple_field_kind(field: &XmlElement) -> Option<RunFieldKind> {
    field
        .attributes
        .iter()
        .find(|attribute| attribute.name == "w:instr")
        .and_then(|instr| DocumentParser::simple_field_kind(&instr.value))
        .map(|kind| {
            kind.with_lock(field.attributes.iter().any(|attribute| {
                matches!(attribute.name.as_str(), "w:fldLock" | "fldLock")
                    && matches!(
                        attribute.value.trim().to_ascii_lowercase().as_str(),
                        "true" | "1" | "on"
                    )
            }))
        })
}

/// Whether `run` holds a `w:ruby` (ECMA-376 Part 1 §17.3.3.25) rather than
/// text of its own.
pub fn is_ruby_run(run: &XmlElement) -> bool {
    element_children(run).any(|child| child.name == "w:ruby")
}

/// Whether a paragraph-level element is a wrapper whose child runs map to
/// model runs. The writer walks the same set when it edits those runs in place.
pub fn is_transparent_run_container(name: &str) -> bool {
    // Preservation bridge only: these wrappers remain raw OOXML containers while
    // their child runs are editable. Full support for each element should move
    // semantics into parser/model/layout/writer code and retire this fallback.
    matches!(
        name,
        "w:hyperlink"
            | "w:smartTag"
            | "w:customXml"
            | "w:sdt"
            | "w:sdtContent"
            | "w:fldSimple"
            | "w:dir"
            | "w:bdo"
    )
}

fn collect_transparently_wrapped_children<'a>(
    nodes: &'a [XmlNode],
    expected_names: &[&str],
    out: &mut Vec<&'a XmlElement>,
) {
    for node in nodes {
        let XmlNode::Element(element) = node else {
            continue;
        };
        if expected_names.contains(&element.name.as_str()) {
            out.push(element);
        } else if is_transparent_block_container(&element.name) {
            // These elements preserve control, binding, or revision semantics
            // in raw OOXML while wrapping block/row/cell nodes the model owns.
            // Recurse only through known transparent containers so a table's
            // nested paragraphs are not mistaken for body siblings.
            collect_transparently_wrapped_children(&element.children, expected_names, out);
        }
    }
}

fn is_transparent_block_container(name: &str) -> bool {
    matches!(
        name,
        "w:sdt" | "w:sdtContent" | "w:customXml" | "w:ins" | "w:del" | "w:moveFrom" | "w:moveTo"
    )
}

fn map_table(table: &mut Table, xml_table: &XmlElement, doc: &DocxXmlDocument) {
    table.node_id = Some(xml_table.id);

    let mut rows = Vec::new();
    collect_transparently_wrapped_children(&xml_table.children, &["w:tr"], &mut rows);
    let mut xml_rows = rows.into_iter();
    for row in &mut table.rows {
        if let Some(xml_row) = xml_rows.next() {
            map_table_row(row, xml_row, doc);
        } else {
            row.node_id = None;
            for cell in &mut row.cells {
                cell.node_id = None;
                clear_block_ids(cell.blocks.as_mut_slice());
            }
        }
    }
}

fn map_table_row(row: &mut TableRow, xml_row: &XmlElement, doc: &DocxXmlDocument) {
    row.node_id = Some(xml_row.id);
    let mut cells = Vec::new();
    collect_transparently_wrapped_children(&xml_row.children, &["w:tc"], &mut cells);
    let mut xml_cells = cells.into_iter();
    for cell in &mut row.cells {
        if let Some(xml_cell) = xml_cells.next() {
            map_table_cell(cell, xml_cell, doc);
        } else {
            cell.node_id = None;
            clear_block_ids(cell.blocks.as_mut_slice());
        }
    }
}

fn map_table_cell(cell: &mut TableCell, xml_cell: &XmlElement, doc: &DocxXmlDocument) {
    cell.node_id = Some(xml_cell.id);
    map_body_elements(cell.blocks.as_mut_slice(), xml_cell, doc);
}

fn map_text_box_elements(elements: &mut [BodyElement], xml: &DocxXmlDocument) {
    for element in elements {
        match element {
            BodyElement::Paragraph(paragraph) => {
                map_text_box_in_paragraph(paragraph.as_mut(), xml);
            }
            BodyElement::Table(table) => {
                let table = table.as_mut();
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        map_text_box_elements(cell.blocks.as_mut_slice(), xml);
                    }
                }
            }
            BodyElement::Math(_) => {}
        }
    }
}

fn map_text_box_in_paragraph(paragraph: &mut Paragraph, xml: &DocxXmlDocument) {
    for run in &mut paragraph.runs {
        let Some(text_box) = run.text_box.as_mut() else {
            continue;
        };
        let Some(run_id) = run.node_id else {
            clear_block_ids(text_box.blocks.as_mut_slice());
            continue;
        };
        let Some(run_element) = xml.element(run_id) else {
            clear_block_ids(text_box.blocks.as_mut_slice());
            continue;
        };
        let mut contents = Vec::new();
        collect_text_box_contents(run_element, &mut contents);
        if let Some(primary) = contents.into_iter().next() {
            map_body_elements(text_box.blocks.as_mut_slice(), primary, xml);
        } else {
            clear_block_ids(text_box.blocks.as_mut_slice());
        }
    }
}

fn collect_text_box_contents<'a>(element: &'a XmlElement, out: &mut Vec<&'a XmlElement>) {
    if element.name == "w:txbxContent" {
        out.push(element);
    }
    for child in element.children.iter() {
        if let XmlNode::Element(child_element) = child {
            collect_text_box_contents(child_element, out);
        }
    }
}

fn find_body(xml: &DocxXmlDocument) -> Option<&XmlElement> {
    element_children(xml.root()).find(|element| element.name == "w:body")
}

fn element_children(element: &XmlElement) -> impl Iterator<Item = &XmlElement> {
    element.children.iter().filter_map(|node| match node {
        XmlNode::Element(child) => Some(child),
        _ => None,
    })
}

fn find_next_with_name<'a>(
    iter: &mut impl Iterator<Item = &'a XmlElement>,
    name: &str,
) -> Option<&'a XmlElement> {
    iter.find(|element| element.name == name)
}

fn clear_block_ids(blocks: &mut [BodyElement]) {
    for block in blocks {
        match block {
            BodyElement::Paragraph(paragraph) => {
                let paragraph = paragraph.as_mut();
                paragraph.node_id = None;
                for run in &mut paragraph.runs {
                    run.node_id = None;
                    run.text_node_id = None;
                    run.revision_node_id = None;
                    if let Some(field) = run.field.as_mut() {
                        field.source_field_id = None;
                    }
                }
            }
            BodyElement::Table(table) => {
                let table = table.as_mut();
                table.node_id = None;
                for row in &mut table.rows {
                    row.node_id = None;
                    for cell in &mut row.cells {
                        cell.node_id = None;
                        clear_block_ids(cell.blocks.as_mut_slice());
                    }
                }
            }
            BodyElement::Math(_) => {
                // Math paragraphs currently do not carry XML node IDs.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Body;
    use crate::parser::DocumentParser;

    #[test]
    fn maps_paragraphs_after_math_wrappers_to_their_own_xml_nodes() {
        const XML: &str = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><w:body>
<w:p><w:r><w:t>Before</w:t></w:r></w:p>
<w:p><m:oMathPara><m:oMath><m:r><m:t>x</m:t></m:r></m:oMath></m:oMathPara></w:p>
<w:p><w:r><w:t>After</w:t></w:r></w:p>
<w:p><m:oMathPara><m:oMath><m:r><m:t>y</m:t></m:r></m:oMath></m:oMathPara><w:r><w:t>Mixed</w:t></w:r></w:p>
<w:p><w:r><w:t>Last</w:t></w:r></w:p>
</w:body></w:document>"#;
        let mut document = DocumentParser::new()
            .parse_document_xml(XML)
            .expect("parse");
        let xml = DocxXmlDocument::parse(XML).expect("raw XML");
        attach_document_xml_ids(&mut document, &xml).expect("map ids");
        let xml_body = element_children(xml.root())
            .find(|element| element.name == "w:body")
            .expect("XML body");
        let xml_paragraphs: Vec<_> = element_children(xml_body)
            .filter(|element| element.name == "w:p")
            .collect();
        let body = document.body.expect("body");
        let mapped_ids: Vec<_> = body
            .elements
            .iter()
            .filter_map(|element| element.as_paragraph().map(|paragraph| paragraph.node_id))
            .collect();
        assert_eq!(
            mapped_ids,
            vec![
                Some(xml_paragraphs[0].id),
                Some(xml_paragraphs[2].id),
                Some(xml_paragraphs[3].id),
                Some(xml_paragraphs[4].id),
            ]
        );
    }

    #[test]
    fn empty_result_simple_field_does_not_take_the_next_runs_element() {
        const XML: &str = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
<w:p><w:r><w:t>ab</w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r/></w:fldSimple><w:r><w:t>cd</w:t></w:r></w:p>
<w:p><w:fldSimple w:instr=" PAGE "/><w:fldSimple w:instr=" PAGE "></w:fldSimple><w:r><w:t>ef</w:t></w:r></w:p>
</w:body></w:document>"#;
        let mut document = DocumentParser::new()
            .parse_document_xml(XML)
            .expect("parse");
        let xml = DocxXmlDocument::parse(XML).expect("raw XML");
        attach_document_xml_ids(&mut document, &xml).expect("map ids");
        let xml_body = find_body(&xml).expect("XML body");
        let xml_paragraphs: Vec<_> = element_children(xml_body).collect();
        let body = document.body.expect("body");
        let paragraphs: Vec<_> = body
            .elements
            .iter()
            .filter_map(BodyElement::as_paragraph)
            .collect();

        // The parser stands a zero-width PAGE run in for the field; it has no
        // element of its own, so it stays unbound and names its container.
        let xml_children: Vec<_> = element_children(xml_paragraphs[0]).collect();
        let runs = &paragraphs[0].runs;
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].node_id, Some(xml_children[0].id));
        assert_eq!(runs[1].text, "");
        assert_eq!(runs[1].node_id, None);
        let page = runs[1].field.as_ref().expect("PAGE field");
        assert_eq!(page.kind, RunFieldKind::PageNumber);
        assert_eq!(page.source_field_id, Some(xml_children[1].id));
        assert_eq!(runs[2].text, "cd");
        assert_eq!(runs[2].node_id, Some(xml_children[2].id));

        // Self-closing and expanded forms each retain one semantic run and
        // their own wrapper; neither may consume the following text's node.
        let xml_children: Vec<_> = element_children(xml_paragraphs[1]).collect();
        let runs = &paragraphs[1].runs;
        assert_eq!(runs.len(), 3);
        for index in [0, 1] {
            assert_eq!(runs[index].node_id, None);
            assert_eq!(
                runs[index]
                    .field
                    .as_ref()
                    .and_then(|field| field.source_field_id),
                Some(xml_children[index].id)
            );
        }
        assert_eq!(runs[2].text, "ef");
        assert_eq!(runs[2].node_id, Some(xml_children[2].id));
    }

    const SAMPLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:r><w:t>Paragraph</w:t></w:r>
    </w:p>
    <w:tbl>
      <w:tr>
        <w:tc>
          <w:p>
            <w:r><w:t>Cell</w:t></w:r>
          </w:p>
        </w:tc>
      </w:tr>
    </w:tbl>
  </w:body>
</w:document>
"#;

    #[test]
    fn maps_paragraph_and_table_nodes() {
        let mut document = DocumentParser::new()
            .parse_document_xml(SAMPLE_XML)
            .expect("parse sample document");
        let xml = DocxXmlDocument::parse(SAMPLE_XML).expect("parse raw xml");
        attach_document_xml_ids(&mut document, &xml).expect("map ids");

        let body: Body = document.body.expect("body");
        assert_eq!(body.elements.len(), 2);

        let xml_body = super::element_children(xml.root())
            .find(|elem| elem.name == "w:body")
            .expect("xml body");

        let xml_paragraph = super::element_children(xml_body)
            .find(|elem| elem.name == "w:p")
            .expect("xml paragraph");
        let xml_run = super::element_children(xml_paragraph)
            .find(|elem| elem.name == "w:r")
            .expect("xml run");
        let xml_text = super::element_children(xml_run)
            .find(|elem| elem.name == "w:t")
            .expect("xml text");

        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        assert_eq!(paragraph.node_id, Some(xml_paragraph.id));
        let run = paragraph.runs.first().expect("run");
        assert_eq!(run.node_id, Some(xml_run.id));
        assert_eq!(run.text_node_id, Some(xml_text.id));

        let xml_table = super::element_children(xml_body)
            .find(|elem| elem.name == "w:tbl")
            .expect("xml table");
        let xml_row = super::element_children(xml_table)
            .find(|elem| elem.name == "w:tr")
            .expect("xml row");
        let xml_cell = super::element_children(xml_row)
            .find(|elem| elem.name == "w:tc")
            .expect("xml cell");
        let xml_cell_paragraph = super::element_children(xml_cell)
            .find(|elem| elem.name == "w:p")
            .expect("xml cell paragraph");
        let xml_cell_run = super::element_children(xml_cell_paragraph)
            .find(|elem| elem.name == "w:r")
            .expect("xml cell run");
        let xml_cell_text = super::element_children(xml_cell_run)
            .find(|elem| elem.name == "w:t")
            .expect("xml cell text");

        let table = match &body.elements[1] {
            BodyElement::Table(t) => t.as_ref(),
            _ => panic!("expected table"),
        };
        assert_eq!(table.node_id, Some(xml_table.id));
        let row = table.rows.first().expect("row");
        assert_eq!(row.node_id, Some(xml_row.id));
        let cell = row.cells.first().expect("cell");
        assert_eq!(cell.node_id, Some(xml_cell.id));

        let cell_paragraph = match &cell.blocks[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph in cell"),
        };
        assert_eq!(cell_paragraph.node_id, Some(xml_cell_paragraph.id));
        let cell_run = cell_paragraph.runs.first().expect("cell run");
        assert_eq!(cell_run.node_id, Some(xml_cell_run.id));
        assert_eq!(cell_run.text_node_id, Some(xml_cell_text.id));
    }

    #[test]
    fn maps_block_table_row_and_cell_nodes_inside_sdt_content() {
        const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:sdt>
      <w:sdtPr><w:tag w:val="ControlledBlock"/></w:sdtPr>
      <w:sdtContent>
        <w:p><w:r><w:t>Controlled paragraph</w:t></w:r></w:p>
        <w:tbl>
          <w:sdt>
            <w:sdtContent>
              <w:tr>
                <w:sdt>
                  <w:sdtContent>
                    <w:tc>
                      <w:sdt>
                        <w:sdtContent>
                          <w:p><w:r><w:t>Controlled cell</w:t></w:r></w:p>
                        </w:sdtContent>
                      </w:sdt>
                    </w:tc>
                  </w:sdtContent>
                </w:sdt>
              </w:tr>
            </w:sdtContent>
          </w:sdt>
        </w:tbl>
      </w:sdtContent>
    </w:sdt>
  </w:body>
</w:document>
"#;

        let mut document = DocumentParser::new()
            .parse_document_xml(XML)
            .expect("parse SDT document");
        let xml = DocxXmlDocument::parse(XML).expect("parse SDT raw xml");
        attach_document_xml_ids(&mut document, &xml).expect("map SDT ids");

        let xml_body = super::element_children(xml.root())
            .find(|element| element.name == "w:body")
            .expect("xml body");
        let block_sdt = super::element_children(xml_body)
            .find(|element| element.name == "w:sdt")
            .expect("block sdt");
        let block_content = super::element_children(block_sdt)
            .find(|element| element.name == "w:sdtContent")
            .expect("block sdtContent");
        let xml_paragraph = super::element_children(block_content)
            .find(|element| element.name == "w:p")
            .expect("controlled paragraph");
        let xml_table = super::element_children(block_content)
            .find(|element| element.name == "w:tbl")
            .expect("controlled table");
        let row_sdt = super::element_children(xml_table)
            .find(|element| element.name == "w:sdt")
            .expect("row sdt");
        let row_content = super::element_children(row_sdt)
            .find(|element| element.name == "w:sdtContent")
            .expect("row sdtContent");
        let xml_row = super::element_children(row_content)
            .find(|element| element.name == "w:tr")
            .expect("controlled row");
        let cell_sdt = super::element_children(xml_row)
            .find(|element| element.name == "w:sdt")
            .expect("cell sdt");
        let cell_content = super::element_children(cell_sdt)
            .find(|element| element.name == "w:sdtContent")
            .expect("cell sdtContent");
        let xml_cell = super::element_children(cell_content)
            .find(|element| element.name == "w:tc")
            .expect("controlled cell");
        let cell_block_sdt = super::element_children(xml_cell)
            .find(|element| element.name == "w:sdt")
            .expect("cell block sdt");
        let cell_block_content = super::element_children(cell_block_sdt)
            .find(|element| element.name == "w:sdtContent")
            .expect("cell block sdtContent");
        let xml_cell_paragraph = super::element_children(cell_block_content)
            .find(|element| element.name == "w:p")
            .expect("controlled cell paragraph");

        let body = document.body.expect("body");
        let paragraph = body.elements[0]
            .as_paragraph()
            .expect("model controlled paragraph");
        assert_eq!(paragraph.node_id, Some(xml_paragraph.id));
        let table = match &body.elements[1] {
            BodyElement::Table(table) => table.as_ref(),
            _ => panic!("expected model controlled table"),
        };
        assert_eq!(table.node_id, Some(xml_table.id));
        let row = table.rows.first().expect("model controlled row");
        assert_eq!(row.node_id, Some(xml_row.id));
        let cell = row.cells.first().expect("model controlled cell");
        assert_eq!(cell.node_id, Some(xml_cell.id));
        let cell_paragraph = cell.blocks[0]
            .as_paragraph()
            .expect("model controlled cell paragraph");
        assert_eq!(cell_paragraph.node_id, Some(xml_cell_paragraph.id));
    }

    #[test]
    fn maps_sdt_nodes_through_custom_xml_and_revision_wrappers() {
        const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:customXml w:element="Clause" w:uri="urn:contract">
      <w:sdt><w:sdtContent>
        <w:p><w:r><w:t>Nested controlled paragraph</w:t></w:r></w:p>
      </w:sdtContent></w:sdt>
    </w:customXml>
    <w:tbl>
      <w:ins w:id="1" w:author="Reviewer">
        <w:sdt><w:sdtContent>
          <w:tr>
            <w:customXml w:element="Cell" w:uri="urn:contract">
              <w:sdt><w:sdtContent>
                <w:tc><w:p><w:r><w:t>Nested controlled cell</w:t></w:r></w:p></w:tc>
              </w:sdtContent></w:sdt>
            </w:customXml>
          </w:tr>
        </w:sdtContent></w:sdt>
      </w:ins>
    </w:tbl>
  </w:body>
</w:document>
"#;

        let mut document = DocumentParser::new()
            .parse_document_xml(XML)
            .expect("parse composed SDT document");
        let xml = DocxXmlDocument::parse(XML).expect("parse composed SDT raw xml");
        attach_document_xml_ids(&mut document, &xml).expect("map composed SDT ids");

        // Word freely composes content controls with custom XML mapping and
        // tracked block revisions. Those outer wrappers must not make the
        // editable descendants look detached from the preserved raw package.
        let body = document.body.expect("body");
        let paragraph = body.elements[0]
            .as_paragraph()
            .expect("nested controlled paragraph");
        assert!(paragraph.node_id.is_some());
        assert!(paragraph.runs[0].node_id.is_some());

        let table = match &body.elements[1] {
            BodyElement::Table(table) => table.as_ref(),
            _ => panic!("expected table"),
        };
        assert!(table.node_id.is_some());
        let row = table.rows.first().expect("nested controlled row");
        assert!(row.node_id.is_some());
        let cell = row.cells.first().expect("nested controlled cell");
        assert!(cell.node_id.is_some());
        let cell_paragraph = cell.blocks[0]
            .as_paragraph()
            .expect("nested controlled cell paragraph");
        assert!(cell_paragraph.node_id.is_some());
        assert!(cell_paragraph.runs[0].node_id.is_some());
    }

    #[test]
    fn maps_runs_inside_transparent_wrapper_nodes() {
        const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:hyperlink w:anchor="Bookmark1">
        <w:r><w:t>Link</w:t></w:r>
      </w:hyperlink>
      <w:smartTag w:uri="urn:test" w:element="term">
        <w:r><w:t>Smart</w:t></w:r>
      </w:smartTag>
      <w:customXml w:element="Clause" w:uri="urn:contract">
        <w:r><w:t>Custom</w:t></w:r>
      </w:customXml>
      <w:sdt>
        <w:sdtContent>
          <w:r><w:t>Content</w:t></w:r>
        </w:sdtContent>
      </w:sdt>
      <w:fldSimple w:instr="AUTHOR">
        <w:r><w:t>Field</w:t></w:r>
      </w:fldSimple>
      <w:dir w:val="rtl">
        <w:r><w:t>Dir</w:t></w:r>
      </w:dir>
      <w:bdo w:val="rtl">
        <w:r><w:t>Bdo</w:t></w:r>
      </w:bdo>
    </w:p>
  </w:body>
</w:document>
"#;

        let mut document = DocumentParser::new()
            .parse_document_xml(XML)
            .expect("parse transparent wrapper document");
        let xml = DocxXmlDocument::parse(XML).expect("parse raw xml");
        attach_document_xml_ids(&mut document, &xml).expect("map ids");

        let body = document.body.expect("body");
        let paragraph = match &body.elements[0] {
            BodyElement::Paragraph(p) => p.as_ref(),
            _ => panic!("expected paragraph"),
        };
        assert_eq!(paragraph.runs.len(), 7);

        let xml_body = super::element_children(xml.root())
            .find(|elem| elem.name == "w:body")
            .expect("xml body");
        let xml_paragraph = super::element_children(xml_body)
            .find(|elem| elem.name == "w:p")
            .expect("xml paragraph");
        let xml_hyperlink = super::element_children(xml_paragraph)
            .find(|elem| elem.name == "w:hyperlink")
            .expect("xml hyperlink");
        let xml_hyperlink_run = super::element_children(xml_hyperlink)
            .find(|elem| elem.name == "w:r")
            .expect("xml hyperlink run");
        let xml_smart_tag = super::element_children(xml_paragraph)
            .find(|elem| elem.name == "w:smartTag")
            .expect("xml smartTag");
        let xml_smart_tag_run = super::element_children(xml_smart_tag)
            .find(|elem| elem.name == "w:r")
            .expect("xml smartTag run");
        let xml_custom_xml = super::element_children(xml_paragraph)
            .find(|elem| elem.name == "w:customXml")
            .expect("xml customXml");
        let xml_custom_xml_run = super::element_children(xml_custom_xml)
            .find(|elem| elem.name == "w:r")
            .expect("xml customXml run");
        let xml_sdt = super::element_children(xml_paragraph)
            .find(|elem| elem.name == "w:sdt")
            .expect("xml sdt");
        let xml_sdt_content = super::element_children(xml_sdt)
            .find(|elem| elem.name == "w:sdtContent")
            .expect("xml sdtContent");
        let xml_sdt_run = super::element_children(xml_sdt_content)
            .find(|elem| elem.name == "w:r")
            .expect("xml sdt run");
        let xml_fld_simple = super::element_children(xml_paragraph)
            .find(|elem| elem.name == "w:fldSimple")
            .expect("xml fldSimple");
        let xml_fld_simple_run = super::element_children(xml_fld_simple)
            .find(|elem| elem.name == "w:r")
            .expect("xml fldSimple run");
        let xml_dir = super::element_children(xml_paragraph)
            .find(|elem| elem.name == "w:dir")
            .expect("xml dir");
        let xml_dir_run = super::element_children(xml_dir)
            .find(|elem| elem.name == "w:r")
            .expect("xml dir run");
        let xml_bdo = super::element_children(xml_paragraph)
            .find(|elem| elem.name == "w:bdo")
            .expect("xml bdo");
        let xml_bdo_run = super::element_children(xml_bdo)
            .find(|elem| elem.name == "w:r")
            .expect("xml bdo run");

        assert_eq!(paragraph.runs[0].node_id, Some(xml_hyperlink_run.id));
        assert_eq!(paragraph.runs[1].node_id, Some(xml_smart_tag_run.id));
        assert_eq!(paragraph.runs[2].node_id, Some(xml_custom_xml_run.id));
        assert_eq!(paragraph.runs[3].node_id, Some(xml_sdt_run.id));
        assert_eq!(paragraph.runs[4].node_id, Some(xml_fld_simple_run.id));
        assert_eq!(paragraph.runs[5].node_id, Some(xml_dir_run.id));
        assert_eq!(paragraph.runs[6].node_id, Some(xml_bdo_run.id));
    }

    /// jlsi/jade#402: the parser does not model `<w:r/>`, so the mapping
    /// skips it and every model run keeps its own `w:r` element (and with it
    /// its own formatting), also inside a field. `<w:r></w:r>` is still a run.
    #[test]
    fn maps_runs_after_self_closing_empty_runs_to_their_own_xml_nodes() {
        const XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r/><w:r><w:rPr><w:b/></w:rPr><w:t>a</w:t></w:r><w:r/><w:r><w:t>b</w:t></w:r></w:p>
    <w:p><w:r></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>a</w:t></w:r><w:r><w:t>b</w:t></w:r></w:p>
    <w:p><w:r><w:t>x</w:t></w:r><w:fldSimple w:instr=" TC &quot;x&quot; "><w:r/></w:fldSimple><w:r><w:rPr><w:b/></w:rPr><w:t>y</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;

        let mut document = DocumentParser::new()
            .parse_document_xml(XML)
            .expect("parse empty run document");
        let xml = DocxXmlDocument::parse(XML).expect("parse raw xml");
        attach_document_xml_ids(&mut document, &xml).expect("map ids");

        let body = document.body.expect("body");
        let mut mapped = 0;
        for element in &body.elements {
            let BodyElement::Paragraph(paragraph) = element else {
                panic!("expected paragraph");
            };
            for run in &paragraph.runs {
                let node_id = run.node_id.expect("every run maps onto an element");
                let xml_run = xml.element(node_id).expect("mapped element");
                assert_eq!(xml_run.name, "w:r");
                // The element carries this run's own text and formatting.
                let text: String = super::element_children(xml_run)
                    .filter(|child| child.name == "w:t")
                    .flat_map(|t| t.children.iter())
                    .filter_map(|node| match node {
                        crate::docx_xml::XmlNode::Text(text) => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(text, run.text);
                let element_is_bold = super::element_children(xml_run)
                    .filter(|child| child.name == "w:rPr")
                    .any(|props| super::element_children(props).any(|child| child.name == "w:b"));
                let run_is_bold =
                    run.properties.as_ref().and_then(|props| props.bold) == Some(true);
                assert_eq!(element_is_bold, run_is_bold, "{:?}", run.text);
                mapped += 1;
            }
        }
        // 10 `w:r` elements, 3 of them `<w:r/>` (not runs); `<w:r></w:r>` is a run.
        assert_eq!(mapped, 2 + 3 + 2);
    }

    const MATH_NAMESPACES: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math""#;

    /// Parses `paragraphs` as a body and maps it onto its raw XML.
    fn mapped_body(paragraphs: &str) -> (Body, DocxXmlDocument) {
        let source =
            format!("<w:document {MATH_NAMESPACES}><w:body>{paragraphs}</w:body></w:document>");
        let mut document = DocumentParser::new()
            .parse_document_xml(&source)
            .expect("parse document");
        let xml = DocxXmlDocument::parse(&source).expect("parse raw xml");
        attach_document_xml_ids(&mut document, &xml).expect("map ids");
        (document.body.expect("body"), xml)
    }

    /// The `w:t` text of every `w:r` in the paragraph, in document order and
    /// through any wrapper, keyed by element id.
    fn xml_runs(xml: &DocxXmlDocument, paragraph: &Paragraph) -> Vec<(XmlNodeId, String)> {
        fn collect(element: &XmlElement, out: &mut Vec<(XmlNodeId, String)>) {
            for child in super::element_children(element) {
                if child.name == "w:r" {
                    let text = super::element_children(child)
                        .filter(|grandchild| grandchild.name == "w:t")
                        .flat_map(|t| t.children.iter())
                        .filter_map(|node| match node {
                            XmlNode::Text(text) => Some(text.as_str()),
                            _ => None,
                        })
                        .collect();
                    out.push((child.id, text));
                } else if child.name != "m:oMath" {
                    collect(child, out);
                }
            }
        }
        let xml_paragraph = xml
            .element(paragraph.node_id.expect("paragraph id"))
            .expect("xml paragraph");
        let mut out = Vec::new();
        collect(xml_paragraph, &mut out);
        out
    }

    fn xml_run_id(runs: &[(XmlNodeId, String)], text: &str) -> XmlNodeId {
        runs.iter()
            .find(|(_, run_text)| run_text == text)
            .unwrap_or_else(|| panic!("xml run {text:?}"))
            .0
    }

    /// A direct `m:oMath` child (inline equation) is a model run with no
    /// `w:r`. Pairing it with the next `w:r` shifted every later run onto its
    /// left neighbour's element: the equation took "CD"'s run and "CD" none.
    #[test]
    fn maps_runs_after_inline_equation_to_their_own_xml_nodes() {
        let (body, xml) = mapped_body(concat!(
            "<w:p><w:r><w:t>AB</w:t></w:r><m:oMath><m:r><m:t>x</m:t></m:r></m:oMath>",
            "<w:r><w:t>CD</w:t></w:r></w:p>",
            // The same inside run containers and a revision.
            "<w:p><w:hyperlink w:anchor=\"a\"><m:oMath><m:r><m:t>y</m:t></m:r></m:oMath>",
            "<w:r><w:t>EF</w:t></w:r></w:hyperlink><w:r><w:t>GH</w:t></w:r></w:p>",
            "<w:p><w:ins w:id=\"1\" w:author=\"A\"><m:oMath><m:r><m:t>z</m:t></m:r></m:oMath>",
            "<w:r><w:t>IJ</w:t></w:r></w:ins><w:r><w:t>KL</w:t></w:r></w:p>",
            // `<m:oMath/>` is no run at all; equations side by side are two.
            "<w:p><w:r><w:t>MN</w:t></w:r><m:oMath/><m:oMath><m:r><m:t>u</m:t></m:r></m:oMath>",
            "<m:oMath><m:r><m:t>v</m:t></m:r></m:oMath><w:r><w:t>OP</w:t></w:r></w:p>",
        ));

        let mut checked = 0;
        for element in &body.elements {
            let paragraph = element.as_paragraph().expect("paragraph");
            let runs = xml_runs(&xml, paragraph);
            for run in &paragraph.runs {
                if run.math.is_some() {
                    assert_eq!(run.node_id, None, "an equation has no w:r");
                    assert_eq!(run.text_node_id, None);
                    assert_eq!(run.revision_node_id, None);
                    continue;
                }
                let node_id = run
                    .node_id
                    .unwrap_or_else(|| panic!("{:?} keeps its w:r", run.text));
                assert_eq!(node_id, xml_run_id(&runs, &run.text), "{:?}", run.text);
                let text_id = super::element_children(xml.element(node_id).expect("xml run"))
                    .find(|child| child.name == "w:t")
                    .map(|child| child.id);
                assert_eq!(run.text_node_id, text_id, "{:?}", run.text);
                checked += 1;
            }
        }
        assert_eq!(checked, 8);

        let math_runs: Vec<usize> = body
            .elements
            .iter()
            .map(|element| {
                let paragraph = element.as_paragraph().expect("paragraph");
                paragraph
                    .runs
                    .iter()
                    .filter(|run| run.math.is_some())
                    .count()
            })
            .collect();
        assert_eq!(math_runs, vec![1, 1, 1, 2]);
        // The run after the equation inside `w:ins` keeps its revision wrapper.
        let revised = body.elements[2].as_paragraph().expect("paragraph");
        let ins = super::element_children(
            xml.element(revised.node_id.expect("paragraph id"))
                .expect("xml paragraph"),
        )
        .find(|child| child.name == "w:ins")
        .expect("w:ins");
        assert_eq!(revised.runs[1].revision_node_id, Some(ins.id));
        assert_eq!(revised.runs[2].revision_node_id, None);
    }

    /// A `w:fldSimple` without result runs is a zero-width model run with no
    /// `w:r`, and must not take the next run's element. Self-closing fields
    /// behave identically. TOC entry and blank instructions reserve no run.
    #[test]
    fn maps_runs_after_empty_simple_field_to_their_own_xml_nodes() {
        let (body, xml) = mapped_body(concat!(
            r#"<w:p><w:r><w:t>a</w:t></w:r><w:fldSimple w:instr=" PAGE "></w:fldSimple>"#,
            r#"<w:r><w:t>b</w:t></w:r></w:p>"#,
            // `<w:r/>` is no run, so this field has no result run either.
            r#"<w:p><w:r><w:t>c</w:t></w:r><w:fldSimple w:instr=" MERGEFIELD Name "><w:r/></w:fldSimple>"#,
            r#"<w:r><w:t>d</w:t></w:r></w:p>"#,
            r#"<w:p><w:r><w:t>e</w:t></w:r><w:fldSimple w:instr=" PAGE "/>"#,
            r#"<w:fldSimple w:instr=" TC &quot;x&quot; "></w:fldSimple><w:fldSimple w:instr=" "></w:fldSimple>"#,
            // A field whose result run, like PAGE's, is empty in the model.
            r#"<w:fldSimple w:instr=" NUMPAGES "><w:r><w:t>9</w:t></w:r></w:fldSimple>"#,
            r#"<w:r><w:t>f</w:t></w:r></w:p>"#,
        ));

        let paragraphs: Vec<&Paragraph> = body
            .elements
            .iter()
            .map(|element| element.as_paragraph().expect("paragraph"))
            .collect();
        let shapes: Vec<Vec<(&str, bool)>> = paragraphs
            .iter()
            .map(|paragraph| {
                paragraph
                    .runs
                    .iter()
                    .map(|run| (run.text.as_str(), run.field.is_some()))
                    .collect()
            })
            .collect();
        assert_eq!(
            shapes,
            vec![
                vec![("a", false), ("", true), ("b", false)],
                vec![("c", false), ("", true), ("d", false)],
                vec![("e", false), ("", true), ("", true), ("f", false)],
            ]
        );

        for paragraph in &paragraphs[..2] {
            let runs = xml_runs(&xml, paragraph);
            let field = &paragraph.runs[1];
            assert_eq!(field.node_id, None, "a result-less field has no w:r");
            assert_eq!(field.text_node_id, None);
            let source_field = super::element_children(
                xml.element(paragraph.node_id.expect("paragraph id"))
                    .expect("XML paragraph"),
            )
            .find(|element| element.name == "w:fldSimple")
            .expect("source field");
            assert_eq!(
                field.field.as_ref().expect("field").source_field_id,
                Some(source_field.id)
            );
            for run in [&paragraph.runs[0], &paragraph.runs[2]] {
                assert_eq!(
                    run.node_id,
                    Some(xml_run_id(&runs, &run.text)),
                    "{:?}",
                    run.text
                );
            }
        }

        let runs = xml_runs(&xml, paragraphs[2]);
        assert_eq!(paragraphs[2].runs[0].node_id, Some(xml_run_id(&runs, "e")));
        let page = &paragraphs[2].runs[1];
        assert_eq!(page.node_id, None);
        let page_wrapper = element_children(xml.element(paragraphs[2].node_id.unwrap()).unwrap())
            .find(|element| element.name == "w:fldSimple")
            .unwrap();
        assert_eq!(
            page.field.as_ref().unwrap().source_field_id,
            Some(page_wrapper.id)
        );
        let numpages = &paragraphs[2].runs[2];
        let numpages_run = xml_run_id(&runs, "9");
        assert_eq!(numpages.node_id, Some(numpages_run));
        assert_eq!(
            numpages.field.as_ref().expect("field").source_field_id,
            xml.parent_id(numpages_run)
        );
        assert_eq!(paragraphs[2].runs[3].node_id, Some(xml_run_id(&runs, "f")));
    }

    /// Word writes `w:object` inside a `w:r`, but the parser also models one
    /// outside a run as an image run with no `w:r`. One without image data is
    /// no run, and reserves nothing.
    #[test]
    fn maps_runs_after_object_outside_a_run_to_their_own_xml_nodes() {
        const OBJECT: &str = r#"<w:object xmlns:v="urn:schemas-microsoft-com:vml" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><v:shape><v:imagedata r:id="rId1"/></v:shape></w:object>"#;
        let (body, xml) = mapped_body(&format!(
            "<w:p><w:r><w:t>a</w:t></w:r>{OBJECT}<w:r><w:t>b</w:t></w:r><w:object><v:shape xmlns:v=\"urn:schemas-microsoft-com:vml\"/></w:object><w:r><w:t>c</w:t></w:r></w:p>"
        ));
        let paragraph = body.elements[0].as_paragraph().expect("paragraph");
        let shapes: Vec<(&str, bool)> = paragraph
            .runs
            .iter()
            .map(|run| (run.text.as_str(), run.image.is_some()))
            .collect();
        assert_eq!(
            shapes,
            vec![("a", false), ("", true), ("b", false), ("c", false)]
        );
        let runs = xml_runs(&xml, paragraph);
        assert_eq!(paragraph.runs[1].node_id, None, "the object has no w:r");
        for run in [&paragraph.runs[0], &paragraph.runs[2], &paragraph.runs[3]] {
            assert_eq!(
                run.node_id,
                Some(xml_run_id(&runs, &run.text)),
                "{:?}",
                run.text
            );
        }
    }

    /// `<w:r/>` is no run, so a math wrapper paragraph holding only that
    /// still produces no editable paragraph, and the next paragraph keeps
    /// its own element.
    #[test]
    fn math_wrapper_with_only_self_closing_run_is_not_a_paragraph() {
        let (body, xml) = mapped_body(concat!(
            "<w:p><m:oMathPara><m:oMath><m:r><m:t>x</m:t></m:r></m:oMath></m:oMathPara><w:r/></w:p>",
            "<w:p><w:r><w:t>After</w:t></w:r></w:p>",
        ));
        let paragraph = body.elements[1].as_paragraph().expect("paragraph");
        assert_eq!(paragraph.runs[0].text, "After");
        let runs = xml_runs(&xml, paragraph);
        assert_eq!(paragraph.runs[0].node_id, Some(xml_run_id(&runs, "After")));
    }

    /// The parser models the runs inside `w:rubyBase`, not the `w:r` that
    /// holds the `w:ruby`. Each base run maps onto its own element, and so
    /// does the run after the ruby rather than inheriting a base run's.
    #[test]
    fn maps_ruby_base_runs_to_their_own_xml_nodes() {
        const XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p><w:r><w:ruby><w:rubyPr><w:hps w:val="10"/></w:rubyPr><w:rt><w:r><w:t>kana</w:t></w:r></w:rt><w:rubyBase><w:r><w:t>X</w:t></w:r><w:r><w:t>Y</w:t></w:r></w:rubyBase></w:ruby></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>after</w:t></w:r></w:p>
  </w:body>
</w:document>
"#;

        let mut document = DocumentParser::new()
            .parse_document_xml(XML)
            .expect("parse ruby document");
        let xml = DocxXmlDocument::parse(XML).expect("parse raw xml");
        attach_document_xml_ids(&mut document, &xml).expect("map ids");

        let xml_body = super::element_children(xml.root())
            .find(|element| element.name == "w:body")
            .expect("xml body");
        let xml_paragraph = super::element_children(xml_body)
            .find(|element| element.name == "w:p")
            .expect("xml paragraph");
        let xml_runs: Vec<_> = super::element_children(xml_paragraph)
            .filter(|element| element.name == "w:r")
            .collect();
        assert!(super::is_ruby_run(xml_runs[0]));
        let xml_ruby = super::element_children(xml_runs[0])
            .find(|element| element.name == "w:ruby")
            .expect("xml ruby");
        let xml_base = super::element_children(xml_ruby)
            .find(|element| element.name == "w:rubyBase")
            .expect("xml ruby base");
        let xml_base_runs: Vec<_> = super::element_children(xml_base).collect();

        let body = document.body.expect("body");
        let paragraph = body.elements[0].as_paragraph().expect("paragraph");
        let mapped: Vec<_> = paragraph.runs.iter().map(|run| run.node_id).collect();
        assert_eq!(
            mapped,
            vec![
                Some(xml_base_runs[0].id),
                Some(xml_base_runs[1].id),
                Some(xml_runs[1].id),
            ]
        );
    }
}
