//! Native editor integration for upstream DOCX identity bootstrap.
use crate::word_document::WordDocument;
pub use jade_docx_crdt::nodes::{NodeIdentity, NodeManifest};
impl WordDocument {
    pub fn prepare_crdt_nodes(&mut self) -> Result<NodeManifest, String> {
        self.ensure_roundtrip_xml().map_err(|e| e.to_string())?;
        jade_docx_crdt::nodes::prepare_nodes(
            self.raw_document
                .as_mut()
                .ok_or("DOCX XML is unavailable")?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
    fn attr<'a>(
        element: &'a jade_docx_reader::docx_xml::XmlElement,
        name: &str,
    ) -> Option<&'a str> {
        element
            .attributes
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.value.as_str())
    }
    use crate::Options;
    use jade_docx_reader::docx_xml::DocxXmlDocument;
    use std::io::{Cursor, Write};

    const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
    const NS: &str = "urn:jade:crdt:v1";

    fn package(xml: &str) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, content) in [
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            ),
            ("word/document.xml", xml),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn load(xml: &str) -> WordDocument {
        futures::executor::block_on(WordDocument::load(&package(xml), &Options::default())).unwrap()
    }

    fn xml(body: &str) -> String {
        format!(r#"<w:document xmlns:w="{W}"><w:body>{body}</w:body></w:document>"#)
    }

    #[test]
    fn crdt_docx_ids_survive_export_reload_and_seed_real_crdt() {
        let mut doc = load(&xml(
            r#"<w:p><w:r><w:t>許可😀</w:t><w:t>二</w:t></w:r><w:del w:id="1" w:author="A"><w:r><w:delText>旧文</w:delText></w:r></w:del></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>表</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/>"#,
        ));
        let manifest = doc.prepare_crdt_nodes().unwrap();
        assert_eq!(
            manifest
                .nodes
                .iter()
                .filter(|n| n.kind == "paragraph")
                .count(),
            3
        );
        assert_eq!(manifest.nodes.iter().filter(|n| n.kind == "run").count(), 3);
        assert_eq!(
            manifest
                .seeds()
                .iter()
                .map(|n| n.text.as_str())
                .collect::<Vec<_>>(),
            ["許可😀", "二", "旧文", "表"]
        );
        let text = manifest
            .nodes
            .iter()
            .find(|n| n.text.as_deref() == Some("許可😀"))
            .unwrap();
        let run = manifest
            .nodes
            .iter()
            .find(|n| Some(&n.id) == text.parent.as_ref())
            .unwrap();
        assert_eq!(run.kind, "run");
        assert!(run.parent.is_some());
        assert_eq!(manifest, doc.prepare_crdt_nodes().unwrap());
        let saved = doc.export_docx().unwrap();
        let mut reopened =
            futures::executor::block_on(WordDocument::load(&saved, &Options::default())).unwrap();
        assert_eq!(manifest, reopened.prepare_crdt_nodes().unwrap());
        let source = reopened.raw_document.as_ref().unwrap().to_string().unwrap();
        assert!(source.contains("w:delText"));
        assert!(source.contains("jcrdt"));
        let mut a = jade_crdt::Document::create("work", "base", manifest.seeds()).unwrap();
        let mut b = jade_crdt::Document::open("work", "base", &a.snapshot().unwrap()).unwrap();
        let version = a.version().unwrap();
        a.replace(&text.id, 3, 3, "甲").unwrap();
        b.replace(&text.id, 3, 3, "乙").unwrap();
        let (ua, ub) = (
            a.updates_since(&version).unwrap(),
            b.updates_since(&version).unwrap(),
        );
        a.import_updates(&ub).unwrap();
        b.import_updates(&ua).unwrap();
        assert_eq!(a.text(&text.id).unwrap(), b.text(&text.id).unwrap());
    }

    #[test]
    fn crdt_node_ids_ignore_paragraph_order_and_do_not_replace_word_para_id() {
        let mut doc = load(&xml(
            r#"<w:p xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" w14:paraId="12345678"><w:r><w:t>元</w:t></w:r></w:p>"#,
        ));
        let before = doc.prepare_crdt_nodes().unwrap();
        let source = doc.raw_document.as_ref().unwrap().to_string().unwrap();
        assert!(source.contains("12345678"));
        let source = source.replace("<w:body>", "<w:body><w:p><w:r><w:t>追加</w:t></w:r></w:p>");
        doc.raw_document = Some(DocxXmlDocument::parse(&source).unwrap());
        let after = doc.prepare_crdt_nodes().unwrap();
        assert_eq!(before.nodes, after.nodes[3..]);
        assert_ne!(after.nodes[0].id, before.nodes[0].id);
    }

    #[test]
    fn crdt_duplicate_or_invalid_ids_are_rejected_without_mutating_xml() {
        for body in [
            r#"<w:p jcrdt:id="00000000000000000000000000000001"/><w:p jcrdt:id="00000000000000000000000000000001"/>"#,
            r#"<w:p jcrdt:id="not-an-id"/>"#,
        ] {
            let source = xml(body).replace(
                "<w:document ",
                &format!(r#"<w:document xmlns:jcrdt="{NS}" "#),
            );
            let mut doc = load(&source);
            let before = doc.raw_document.as_ref().unwrap().to_string().unwrap();
            assert!(doc.prepare_crdt_nodes().is_err());
            assert_eq!(
                before,
                doc.raw_document.as_ref().unwrap().to_string().unwrap()
            );
        }
    }

    #[test]
    fn crdt_namespace_conflicts_are_rejected_without_mutation() {
        for source in [
            xml("<w:p/>").replace("<w:document ", r#"<w:document xmlns:jcrdt="urn:other" "#),
            xml(r#"<w:p xmlns:jcrdt="urn:other"/>"#),
            xml("<w:p/>").replace("<w:document ", r#"<w:document xmlns:mc="urn:other" "#),
            xml(r#"<w:p jcrdt:id="00000000000000000000000000000001"/>"#),
            xml("<w:p/>").replace(
                "<w:document ",
                &format!(r#"<w:document xmlns:other="{NS}" "#),
            ),
            xml("<w:p/>").replace(
                "<w:document ",
                &format!(r#"<w:document xmlns:jcrdt="{NS}" jcrdt:schema="2" "#),
            ),
        ] {
            let mut doc = load(&source);
            let before = doc.raw_document.as_ref().unwrap().to_string().unwrap();
            assert!(doc.prepare_crdt_nodes().is_err());
            assert_eq!(
                before,
                doc.raw_document.as_ref().unwrap().to_string().unwrap()
            );
        }
    }

    #[test]
    fn crdt_preserves_compatibility_tokens_and_empty_text() {
        let source = xml(r#"<w:p><w:r><w:t/></w:r></w:p>"#).replace("<w:document ", &format!(r#"<w:document xmlns:mc="{MC}" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" mc:Ignorable="w14" "#));
        let mut doc = load(&source);
        let manifest = doc.prepare_crdt_nodes().unwrap();
        assert_eq!(manifest.seeds()[0].text, "");
        assert_eq!(
            attr(doc.raw_document.as_ref().unwrap().root(), "mc:Ignorable"),
            Some("w14 jcrdt")
        );
        assert_eq!(manifest, doc.prepare_crdt_nodes().unwrap());
    }

    #[test]
    fn crdt_lazy_load_can_prepare_without_changing_semantic_text() {
        let bytes = package(&xml(r#"<w:p><w:r><w:t>本文</w:t></w:r></w:p>"#));
        let mut doc = futures::executor::block_on(WordDocument::load_for_initial_render(
            &bytes,
            &Options::default(),
        ))
        .unwrap();
        assert!(doc.raw_document.is_none());
        let before = doc.paragraph(0).unwrap().runs[0].text.clone();
        let manifest = doc.prepare_crdt_nodes().unwrap();
        assert_eq!(manifest.seeds()[0].text, before);
        assert_eq!(doc.paragraph(0).unwrap().runs[0].text, before);
    }
}
