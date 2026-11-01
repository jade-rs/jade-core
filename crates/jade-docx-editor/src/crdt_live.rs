//! Native host policy boundary. The default editor attaches no domain semantics.
pub use jade_docx_crdt::{LiveDocument, LivePosition};

/// Configure metadata derivation before attaching a replica. Hosts may supply
/// a deterministic fact policy here; the generic editor never interprets facts.
pub fn configure(live: LiveDocument) -> LiveDocument {
    live
}

/// Optional host-specific projections. Raw causal facts are always exported.
pub fn projections(_live: &LiveDocument) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!([]))
}

/// Preserve optional host display policy across native document replacement.
pub type DisplayState = ();
pub fn display_state(_doc: &crate::word_document::WordDocument) -> DisplayState {}
pub fn restore_display_state(_doc: &crate::word_document::WordDocument, _saved: DisplayState) {}

#[cfg(test)]
mod tests {
    use super::*;
    fn package(text: &str, binary: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, bytes) in [
            ("[Content_Types].xml",br#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.as_slice()),
            ("_rels/.rels",br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.as_slice()),
            ("word/document.xml",text.as_bytes()),("word/media/image.bin",binary)] {
            zip.start_file(path,zip::write::SimpleFileOptions::default()).unwrap();zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }
    #[test]
    fn live_native_document_roundtrip_preserves_replica_binding() {
        use crate::word_document::WordDocument;
        let options = crate::Options::default();
        let bytes = package(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>料金</w:t></w:r></w:p><w:tbl><w:tblPr/><w:tblGrid><w:gridCol w:w="2000"/></w:tblGrid><w:tr><w:tc><w:tcPr/><w:p><w:r><w:t>表</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:sectPr/></w:body></w:document>"#,
            &[],
        );
        let mut doc = futures::executor::block_on(WordDocument::load(&bytes, &options)).unwrap();
        doc.ensure_roundtrip_xml().unwrap();
        let bytes = doc.export_docx().unwrap();
        let mut live =
            LiveDocument::create("work", "base", &bytes, doc.raw_document.as_ref()).unwrap();
        let projected = live.projected_docx().unwrap();
        let mut reloaded =
            futures::executor::block_on(WordDocument::load(&projected, &options)).unwrap();
        reloaded.ensure_roundtrip_xml().unwrap();
        live.rebind(
            &reloaded.export_docx().unwrap(),
            reloaded.raw_document.as_ref(),
        )
        .unwrap();
        LiveDocument::open(
            "work",
            "base",
            &reloaded.export_docx().unwrap(),
            &live.inner.snapshot().unwrap(),
            reloaded.raw_document.as_ref(),
        )
        .unwrap();
    }
    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config::with_cases(32))]
        #[test]
        fn native_unicode_identity_snapshot_and_history_roundtrip(
            chars in proptest::collection::vec(proptest::sample::select(vec!['a','漢','😀','\u{301}','&','<']),0..32)
        ) {
            use crate::word_document::WordDocument;
            let text: String = chars.into_iter().collect();
            let escaped=text.replace('&',"&amp;").replace('<',"&lt;");
            let source=format!(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>{escaped}</w:t></w:r></w:p></w:body></w:document>"#);
            let options=crate::Options::default();
            let mut doc=futures::executor::block_on(WordDocument::load(&package(&source,&[]),&options)).unwrap();
            let manifest=doc.prepare_crdt_nodes().unwrap();
            proptest::prop_assert_eq!(&manifest,&doc.prepare_crdt_nodes().unwrap());
            let mut live=LiveDocument::create("work","base",&doc.export_docx().unwrap(),doc.raw_document.as_ref()).unwrap();
            let before=live.inner.visible_structure().unwrap();
            use jade_docx_reader::docx_xml::{XmlElement, XmlNode, XmlNodeId};
            fn text_element(e: &XmlElement) -> Option<XmlNodeId> {
                if e.name == "w:t" { return Some(e.id); }
                e.children.iter().find_map(|c| match c {
                    XmlNode::Element(e) => text_element(e),
                    _ => None,
                })
            }
            let main = doc.raw_document.as_mut().unwrap();
            let id = text_element(main.root()).unwrap();
            main.track_changes();
            main.element_mut(id).unwrap().children = vec![XmlNode::Text("置換😀".into())];
            proptest::prop_assert!(live.capture_changed("a", main).unwrap());
            let after=live.inner.visible_structure().unwrap();
            live.history("a",false).unwrap();
            proptest::prop_assert_eq!(live.inner.visible_structure().unwrap(),before);
            live.history("a",true).unwrap();
            proptest::prop_assert_eq!(live.inner.visible_structure().unwrap(),after.clone());
            let mut reloaded=futures::executor::block_on(WordDocument::load(&live.projected_docx().unwrap(),&options)).unwrap();
            let reopened_manifest=reloaded.prepare_crdt_nodes().unwrap();
            proptest::prop_assert_eq!(reopened_manifest.seeds().iter().map(|n|n.id.clone()).collect::<Vec<_>>(),manifest.seeds().iter().map(|n|n.id.clone()).collect::<Vec<_>>());
            live.rebind(&reloaded.export_docx().unwrap(),reloaded.raw_document.as_ref()).unwrap();
            let reopened=LiveDocument::open("work","base",&reloaded.export_docx().unwrap(),&live.inner.snapshot().unwrap(),reloaded.raw_document.as_ref()).unwrap();
            proptest::prop_assert_eq!(reopened.inner.visible_structure().unwrap(),after);
        }
    }
}
