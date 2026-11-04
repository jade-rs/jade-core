//! 表のプロパティ's switches that have no layout command of their own,
//! written where the schema puts them so Word reads the table as written:
//!
//! - セル › オプション › 文字列をセル幅に合わせる (`w:tcFitText`).

use super::*;

/// `w:tcPr`'s children in schema order (ECMA-376 Part 1 §17.4.70).
const CELL_PROPERTY_ORDER: &[&str] = &[
    "w:cnfStyle",
    "w:tcW",
    "w:gridSpan",
    "w:hMerge",
    "w:vMerge",
    "w:tcBorders",
    "w:shd",
    "w:noWrap",
    "w:tcMar",
    "w:textDirection",
    "w:tcFitText",
    "w:vAlign",
    "w:hideMark",
    "w:headers",
    "w:cellIns",
    "w:cellDel",
    "w:cellMerge",
    "w:tcPrChange",
];

fn rank(order: &[&str], name: &str) -> usize {
    order
        .iter()
        .position(|candidate| *candidate == name)
        .unwrap_or(order.len())
}

impl WordDocument {
    /// Puts `name` (with `attributes`, or none at all when `None`) under the
    /// properties element `properties` (`w:tblPr` / `w:tcPr`) of the
    /// element `owner`, at its place in `order`.
    fn put_ordered_property(
        &mut self,
        owner: Option<XmlNodeId>,
        properties: &str,
        order: &[&str],
        name: &str,
        attributes: Option<Vec<XmlAttribute>>,
    ) -> Result<(), String> {
        let Some(owner) = owner else {
            return Ok(());
        };
        let Some(raw) = self.raw_document.as_mut() else {
            return Ok(());
        };
        let properties_id = Self::ensure_properties_element(raw, owner, properties)?;
        let element = attributes.map(|attributes| raw.create_element(name, attributes, Vec::new()));
        let parent = raw
            .element_mut(properties_id)
            .ok_or_else(|| format!("{properties} went missing"))?;
        parent
            .children
            .retain(|node| !matches!(node, XmlNode::Element(existing) if existing.name == name));
        if let Some(element) = element {
            let own = rank(order, name);
            let at = parent
                .children
                .iter()
                .position(|node| {
                    matches!(node, XmlNode::Element(existing) if rank(order, &existing.name) > own)
                })
                .unwrap_or(parent.children.len());
            parent.children.insert(at, XmlNode::Element(element));
        }
        raw.rebuild_index_for_subtree(owner);
        Ok(())
    }

    /// 文字列をセル幅に合わせる: the cell's text squeezed or spread to the
    /// cell's width, or as it is.
    pub fn set_table_cell_fit_text(
        &mut self,
        table_order_index: usize,
        row: usize,
        column: usize,
        fit: bool,
    ) -> Result<(), String> {
        let cell_id = {
            let cell = self.table_cell_mut_at(table_order_index, row, column)?;
            if fit {
                cell.style
                    .insert("fit-text".to_string(), "true".to_string());
            } else {
                cell.style.remove("fit-text");
            }
            cell.node_id
        };
        self.put_ordered_property(
            cell_id,
            "w:tcPr",
            CELL_PROPERTY_ORDER,
            "w:tcFitText",
            fit.then(Vec::new),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::word_document::review_tools::tests::docx_with_body;
    use futures::executor;
    use std::io::Read;

    const TABLE: &str = r#"<w:tbl><w:tblPr><w:tblW w:w="0" w:type="auto"/><w:tblLook w:val="04A0"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:tcW w:w="4000" w:type="dxa"/><w:vAlign w:val="center"/></w:tcPr><w:p><w:r><w:t>氏名</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#;

    fn load(bytes: &[u8]) -> WordDocument {
        executor::block_on(WordDocument::load(bytes, &Options::default())).unwrap()
    }

    fn document_xml(document: &WordDocument) -> (Vec<u8>, String) {
        let saved = document.export_docx().unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&saved)).unwrap();
        let mut xml = String::new();
        archive
            .by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        (saved, xml)
    }

    fn fits(document: &WordDocument) -> bool {
        document.table_by_order(0).unwrap().rows[0].cells[0]
            .style
            .contains_key("fit-text")
    }

    #[test]
    fn fit_text_is_written_in_schema_order_and_read_back() {
        let mut document = load(&docx_with_body(TABLE));
        document.set_table_cell_fit_text(0, 0, 0, true).unwrap();
        assert!(fits(&document));
        let (saved, xml) = document_xml(&document);
        let fit = xml.find("<w:tcFitText/>").expect(&xml);
        assert!(xml.find("<w:tcW ").unwrap() < fit, "{xml}");
        assert!(fit < xml.find("<w:vAlign ").unwrap(), "{xml}");

        let mut reopened = load(&saved);
        assert!(fits(&reopened));
        reopened.set_table_cell_fit_text(0, 0, 0, false).unwrap();
        assert!(!fits(&reopened));
        let (_, xml) = document_xml(&reopened);
        assert!(!xml.contains("tcFitText"), "{xml}");
        assert!(xml.contains("<w:vAlign "), "{xml}");
    }
}
