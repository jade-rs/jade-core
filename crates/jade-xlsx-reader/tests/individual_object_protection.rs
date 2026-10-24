use jade_xlsx_reader::object_protection::*;
const XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
fn drawing(object: &str, client: &str) -> String {
    format!(
        r#"<d:wsDr xmlns:d="{XDR}" xmlns:f="urn:foreign"><d:oneCellAnchor>{object}{client}</d:oneCellAnchor></d:wsDr>"#
    )
}
fn shape(attrs: &str) -> String {
    format!(
        r#"<d:sp {attrs}><d:nvSpPr><d:cNvPr id="7" name="Box"/></d:nvSpPr><d:txBody><a:bodyPr xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"/><a:p xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:r><a:t>old</a:t></a:r></a:p></d:txBody></d:sp>"#
    )
}
#[test]
fn standard_strict_and_unbound_anchors_use_owned_flags_and_schema_defaults() {
    for ns in [
        XDR,
        "http://purl.oclc.org/ooxml/drawingml/spreadsheetDrawing",
        "",
    ] {
        let xml = drawing(
            &shape(r#"fLocksText="false""#),
            r#"<d:clientData fLocksWithSheet=" &#x30; "/>"#,
        )
        .replace(XDR, ns);
        let xml = if ns.is_empty() {
            xml.replace("d:", "").replace("xmlns:d=\"\"", "")
        } else {
            xml
        };
        let source = drawing_source(&xml, 7).unwrap().unwrap();
        assert_eq!(
            source.locks,
            ObjectLocks {
                locked: false,
                text_locked: false,
                ambiguous: false
            }
        );
        assert_eq!(source.name, "Box");
        assert_eq!(
            &xml[source.object_span],
            shape(r#"fLocksText="false""#).replace("d:", if ns.is_empty() { "" } else { "d:" })
        );
    }
    let s = drawing_source(&drawing(&shape(""), "<d:clientData/>"), 7)
        .unwrap()
        .unwrap();
    assert!(s.locks.locked && s.locks.text_locked && !s.locks.ambiguous);
}
#[test]
fn foreign_nested_and_wrong_nonvisual_lookalikes_cannot_unlock_objects() {
    let raw = shape(r#"f:fLocksText="0""#)
        .replace("</d:sp>", r#"<f:clientData fLocksWithSheet="0"/></d:sp>"#);
    let xml = drawing(
        &raw,
        r#"<f:clientData fLocksWithSheet="0"/><d:extLst><d:clientData fLocksWithSheet="0"/></d:extLst>"#,
    );
    let s = drawing_source(&xml, 7).unwrap().unwrap();
    assert!(s.locks.locked && s.locks.text_locked);
    for fake in [
        r#"<f:sp><f:nvSpPr><f:cNvPr id="7"/></f:nvSpPr></f:sp>"#,
        r#"<d:sp><d:extLst><d:cNvPr id="7"/></d:extLst></d:sp>"#,
    ] {
        assert!(
            drawing_source(&drawing(fake, r#"<d:clientData fLocksWithSheet="0"/>"#), 7)
                .unwrap()
                .is_none()
        );
    }
}
#[test]
fn ambiguous_ids_clients_and_malformed_booleans_never_become_permissions() {
    for client in [
        r#"<d:clientData fLocksWithSheet="yes"/>"#,
        r#"<d:clientData fLocksWithSheet="0"/><d:clientData/>"#,
        r#"<d:clientData fLocksWithSheet="0"><f:payload/></d:clientData>"#,
    ] {
        assert!(drawing_source(&drawing(&shape(""), client), 7).is_err());
    }
    assert!(drawing_source(
        &drawing(&shape(r#"fLocksText="bad""#), "<d:clientData/>"),
        7
    )
    .is_err());
    let xml = drawing(&shape(""), r#"<d:clientData fLocksWithSheet="0"/>"#);
    let inner = &xml[xml.find("<d:oneCellAnchor>").unwrap()..xml.find("</d:wsDr>").unwrap()];
    let duplicated = xml.replace("</d:wsDr>", &format!("{inner}</d:wsDr>"));
    assert!(drawing_source(&duplicated, 7).is_err());
}
fn vml(metadata: &str) -> String {
    format!(
        r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel" xmlns:f="urn:foreign"><v:shape id="_x0000_s1025"><x:ClientData ObjectType="Note"><x:Row>1</x:Row><x:Column>2</x:Column>{metadata}</x:ClientData></v:shape></xml>"#
    )
}
#[test]
fn vml_note_flags_use_true_false_blank_with_entities_cdata_and_owned_leaves() {
    for false_value in ["False", "f", "<![CDATA[False]]>", "F&#x61;lse"] {
        let notes = note_sources(&vml(&format!(
            "<x:Locked>{false_value}</x:Locked><x:LockText>False</x:LockText>"
        )))
        .unwrap();
        assert_eq!(notes[0].at, jade_xlsx_reader::CellRef::new(2, 1));
        assert_eq!(
            notes[0].locks,
            ObjectLocks {
                locked: false,
                text_locked: false,
                ambiguous: false
            }
        );
    }
    let n = note_sources(&vml("<x:Locked/><f:LockText>False</f:LockText>")).unwrap();
    assert!(n[0].locks.locked && n[0].locks.text_locked);
    for metadata in [
        "<x:Locked>0</x:Locked>",
        "<x:Locked>False</x:Locked><x:Locked/>",
        "<x:Locked><f:value>False</f:value></x:Locked>",
    ] {
        assert!(note_sources(&vml(metadata)).unwrap()[0].locks.ambiguous);
    }
}
#[test]
fn worksheet_relationships_share_selected_mce_and_relationship_namespace_ownership() {
    let xml = format!(
        r#"<s:worksheet xmlns:s="{MAIN}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:u="urn:unknown"><s:sheetData/><mc:AlternateContent><mc:Choice Requires="u"><s:drawing r:id="wrong"/></mc:Choice><mc:Fallback><s:drawing r:id="chosen"/></mc:Fallback></mc:AlternateContent><s:legacyDrawing r:id="vml"/></s:worksheet>"#
    );
    assert_eq!(
        worksheet_drawing_relationship(&xml, false)
            .unwrap()
            .as_deref(),
        Some("chosen")
    );
    assert_eq!(
        worksheet_drawing_relationship(&xml, true)
            .unwrap()
            .as_deref(),
        Some("vml")
    );
    for root in ["chartsheet", "dialogsheet"] {
        let alternate = xml.replace("worksheet", root);
        assert_eq!(
            worksheet_drawing_relationship(&alternate, false)
                .unwrap()
                .as_deref(),
            Some("chosen")
        );
    }
    let foreign = xml.replace(
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
        "urn:foreign",
    );
    assert!(worksheet_drawing_relationship(&foreign, false).is_err());
    let nested = format!(
        r#"<worksheet xmlns="{MAIN}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><extLst><ext uri="x"><drawing r:id="wrong"/></ext></extLst></worksheet>"#
    );
    assert!(worksheet_drawing_relationship(&nested, false)
        .unwrap()
        .is_none());
}
#[test]
fn comments_read_only_owned_authors_values_and_common_rich_text_projection() {
    let xml = format!(
        r#"<c:comments xmlns:c="{MAIN}" xmlns:f="urn:foreign"><f:authors><f:author>wrong</f:author></f:authors><c:authors><c:author><![CDATA[A & B]]></c:author></c:authors><c:commentList><f:comment ref="A1" authorId="0"><f:text><f:t>wrong</f:t></f:text></f:comment><c:comment ref="A1" authorId="0"><f:text><f:t>wrong</f:t></f:text><c:text><c:r><c:t><![CDATA[value &]]></c:t></c:r><c:rPh sb="0" eb="1"><c:t>phonetic</c:t></c:rPh><c:t>_x0041_</c:t></c:text></c:comment></c:commentList></c:comments>"#
    );
    let notes = jade_xlsx_reader::comments::parse_comments(&xml).unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].author.as_deref(), Some("A & B"));
    assert_eq!(notes[0].text, "value &A");
    assert!(!comments_source(&xml).unwrap().can_drop_empty);
}
#[test]
fn relationship_views_ignore_foreign_nested_records_and_keep_registry_cleanup_conservative() {
    let xml = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships" xmlns:f="urn:foreign"><Relationship Id="real" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing" Target="../drawings/real.xml"/><f:Relationship Id="real" Type="foreign" Target="wrong"/><f:wrapper><Relationship Id="nested" Type="x" Target="wrong"/></f:wrapper></Relationships>"#;
    let r = owned_relationships(xml).unwrap();
    assert_eq!(r.target("real"), Some("../drawings/real.xml"));
    assert!(r.relationship("nested").is_none());
    assert!(!plain_package_registry(xml, false).unwrap());
    let duplicate = xml.replace(
        "</Relationships>",
        r#"<Relationship Id="real" Type="x" Target="wrong"/></Relationships>"#,
    );
    assert!(owned_relationships(&duplicate).is_err());
    let invalid = xml.replace(
        "Target=\"../drawings/real.xml\"",
        "Target=\"../drawings/real.xml\" TargetMode=\"Unknown\"",
    );
    assert!(owned_relationships(&invalid).is_err());
}

#[test]
fn comment_mce_selects_text_authors_and_records_for_reading() {
    let xml = format!(
        r#"<comments xmlns="{MAIN}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:u="urn:unknown"><authors><mc:AlternateContent><mc:Choice Requires="u"><author>wrong</author></mc:Choice><mc:Fallback><author>author</author></mc:Fallback></mc:AlternateContent></authors><commentList><mc:AlternateContent><mc:Choice Requires="x14"><comment ref="A1" authorId="0"><mc:AlternateContent><mc:Choice Requires="u"><text><t>wrong</t></text></mc:Choice><mc:Choice Requires="x14"><text><t>chosen</t></text></mc:Choice><mc:Fallback><text><t>fallback</t></text></mc:Fallback></mc:AlternateContent></comment></mc:Choice><mc:Fallback><comment ref="B2"><text><t>wrong</t></text></comment></mc:Fallback></mc:AlternateContent></commentList></comments>"#
    );
    let notes = jade_xlsx_reader::comments::parse_comments(&xml).unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].reference, jade_xlsx_reader::CellRef::new(0, 0));
    assert_eq!(notes[0].author.as_deref(), Some("author"));
    assert_eq!(notes[0].text, "chosen");
    assert!(comments_source(&xml).is_err());
}
