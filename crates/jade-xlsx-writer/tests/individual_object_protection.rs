use jade_xlsx_reader::{object_protection::*, CellRef};
use jade_xlsx_writer::{delete_shape, set_shape_anchor, set_shape_text, ShapeAnchor};
const XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
fn drawing(kind: &str) -> String {
    format!(
        r#"<d:wsDr xmlns:d="{XDR}" xmlns:f="urn:foreign"><d:{kind} xmlns:q="urn:local" q:keep="x"><d:from><d:col>0</d:col><d:row>0</d:row></d:from><d:ext cx="1" cy="1"/><!--keep--><d:sp fLocksText="0"><d:nvSpPr><d:cNvPr id="7" name="Box"/></d:nvSpPr><d:txBody><a:bodyPr xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"/><a:p xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:pPr algn="r"/><a:r><a:rPr b="1"/><a:t>old</a:t></a:r></a:p><f:p>opaque</f:p></d:txBody></d:sp><d:clientData fLocksWithSheet="0"/><f:payload q:keep="x"/></d:{kind}></d:wsDr>"#
    )
}
fn anchor() -> ShapeAnchor {
    ShapeAnchor {
        from: CellRef::new(1, 2),
        from_offset: (3, 4),
        to: CellRef::new(6, 8),
        to_offset: (5, 6),
    }
}
#[test]
fn anchor_conversion_keeps_local_namespaces_unknown_content_and_both_locks() {
    for kind in ["oneCellAnchor", "twoCellAnchor", "absoluteAnchor"] {
        let old = drawing(kind);
        let xml = set_shape_anchor(&old, 7, &anchor()).unwrap().unwrap();
        let s = drawing_source(&xml, 7).unwrap().unwrap();
        assert_eq!(s.kind, "twoCellAnchor");
        assert!(!s.locks.locked && !s.locks.text_locked);
        assert!(xml.contains(r#"xmlns:q="urn:local" q:keep="x""#));
        assert!(xml.contains("<!--keep-->"));
        assert!(xml.contains(r#"<f:payload q:keep="x"/>"#));
        assert_eq!(
            &xml[s.object_span.clone()],
            &old[drawing_source(&old, 7).unwrap().unwrap().object_span]
        );
        assert!(xml.contains("<d:col>1</d:col>") && xml.contains("<d:row>8</d:row>"));
        if kind != "twoCellAnchor" {
            assert!(xml.contains(if kind == "oneCellAnchor" {
                r#"editAs="oneCell""#
            } else {
                r#"editAs="absolute""#
            }));
        }
    }
}
#[test]
fn drawing_deletion_and_movement_do_not_target_foreign_same_id_records() {
    let opaque = r#"<f:oneCellAnchor><f:sp><f:nvSpPr><f:cNvPr id="7" name="foreign"/></f:nvSpPr></f:sp></f:oneCellAnchor>"#;
    let xml = drawing("oneCellAnchor").replace("</d:wsDr>", &format!("{opaque}</d:wsDr>"));
    assert!(set_shape_anchor(&xml, 7, &anchor())
        .unwrap()
        .unwrap()
        .contains(opaque));
    let out = delete_shape(&xml, 7).unwrap().unwrap();
    assert!(out.contains(opaque));
    assert!(drawing_source(&out, 7).unwrap().is_none());
    let dup = xml.replace(
        "</d:wsDr>",
        &format!(
            "{} </d:wsDr>",
            &xml[drawing_source(&xml, 7).unwrap().unwrap().span]
        ),
    );
    assert!(delete_shape(&dup, 7).is_err());
    assert!(set_shape_text(&dup, 7, "new").is_err());
}
#[test]
fn owned_shape_text_keeps_scoped_fonts_foreign_paragraphs_and_lock_attributes() {
    let xml = drawing("oneCellAnchor");
    let out = set_shape_text(&xml, 7, " new & value\n").unwrap().unwrap();
    let s = drawing_source(&out, 7).unwrap().unwrap();
    assert!(!s.locks.locked && !s.locks.text_locked);
    assert!(out.contains("new &amp; value"));
    assert!(out.contains(r#"xml:space="preserve""#));
    assert!(out.contains(r#"<a:rPr b="1"/>"#));
    assert!(out.contains(r#"<a:endParaRPr b="1"/>"#));
    assert!(out.contains("<f:p>opaque</f:p>"));
    assert!(out.contains(r#"<a:pPr algn="r"/>"#));
    assert!(set_shape_text(&out, 7, "again").unwrap().is_some());
}
fn vml() -> String {
    r#"<xml xmlns:v="urn:schemas-microsoft-com:vml" xmlns:x="urn:schemas-microsoft-com:office:excel" xmlns:f="urn:foreign"><v:shape id="_x0000_s1025" style="custom" f:keep="yes"><x:ClientData ObjectType="Note"><x:Row>0</x:Row><x:Column>0</x:Column><x:Locked>False</x:Locked><x:LockText>False</x:LockText><f:metadata/></x:ClientData></v:shape></xml>"#.into()
}
#[test]
fn updating_existing_note_keeps_vml_byte_for_byte_and_new_shapes_bind_their_prefixes() {
    let xml = vml();
    assert_eq!(
        jade_xlsx_writer::notes::upsert_vml_note(&xml, CellRef::new(0, 0)).unwrap(),
        xml
    );
    for raw in [
        "<xml/>",
        r#"<xml xmlns:v="urn:foreign" xmlns:x="urn:other"/>"#,
    ] {
        let out = jade_xlsx_writer::notes::upsert_vml_note(raw, CellRef::new(2, 3)).unwrap();
        let s = vml_sources(&out).unwrap();
        assert_eq!(s.notes.len(), 1);
        assert_eq!(s.notes[0].at, CellRef::new(2, 3));
        assert!(s.has_note_type);
    }
}
#[test]
fn comment_updates_preserve_foreign_records_root_metadata_and_target_attributes() {
    let foreign = r#"<f:comment ref="A1"><f:text>untouched</f:text></f:comment>"#;
    let xml = format!(
        r#"<s:comments xmlns:s="{MAIN}" xmlns:f="urn:foreign" f:keep="root"><f:authors><f:author>foreign</f:author></f:authors><s:authors><s:author>old</s:author></s:authors><s:commentList>{foreign}<s:comment ref="A1" authorId="0" f:keep="target"><s:text><s:t>old</s:t></s:text><f:payload/></s:comment></s:commentList></s:comments>"#
    );
    let out = jade_xlsx_writer::notes::upsert_comment(
        &xml,
        CellRef::new(0, 0),
        "new & author",
        " new & text ",
    )
    .unwrap();
    assert!(out.contains(foreign));
    assert!(out.contains(r#"f:keep="target""#));
    assert!(out.contains("<f:payload/>"));
    assert!(out.contains(r#"f:keep="root""#));
    let notes = jade_xlsx_reader::comments::parse_comments(&out).unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].author.as_deref(), Some("new & author"));
    assert_eq!(notes[0].text, " new & text ");
}
#[test]
fn self_closing_comment_containers_are_expanded_without_losing_foreign_metadata() {
    let xml = format!(
        r#"<comments xmlns="{MAIN}" xmlns:f="urn:foreign"><authors f:keep="authors"/><commentList f:keep="list"/></comments>"#
    );
    let out = jade_xlsx_writer::notes::upsert_comment(&xml, CellRef::new(1, 1), "author", "body")
        .unwrap();
    assert!(out.contains(r#"f:keep="authors""#) && out.contains(r#"f:keep="list""#));
    assert_eq!(
        jade_xlsx_reader::comments::parse_comments(&out).unwrap()[0].text,
        "body"
    );
}

#[test]
fn comment_text_alternatives_keep_each_font_and_opaque_unsupported_branches() {
    let opaque = r#"<mc:Choice Requires="u"><text><t>opaque</t></text></mc:Choice>"#;
    let xml = format!(
        r#"<comments xmlns="{MAIN}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:x14="http://schemas.microsoft.com/office/spreadsheetml/2009/9/main" xmlns:u="urn:unknown"><authors><author>author</author></authors><commentList><comment ref="A1" authorId="0"><mc:AlternateContent>{opaque}<mc:Choice Requires="x14"><text><r><rPr><b/></rPr><t>chosen</t></r></text></mc:Choice><mc:Fallback><text><r><rPr><i/></rPr><t>fallback</t></r></text></mc:Fallback></mc:AlternateContent></comment></commentList></comments>"#
    );
    let out = jade_xlsx_writer::notes::upsert_comment(
        &xml,
        CellRef::new(0, 0),
        "author",
        " new & value ",
    )
    .unwrap();
    assert!(out.contains(opaque));
    assert!(out.contains("<b/>"));
    assert!(out.contains("<i/>"));
    assert_eq!(out.matches("new &amp; value").count(), 2);
    assert!(!out.contains("chosen"));
    assert!(!out.contains("fallback"));
    assert_eq!(
        jade_xlsx_reader::comments::parse_comments(&out).unwrap()[0].text,
        " new & value "
    );
    let source = jade_xlsx_reader::object_protection::comments_source(&out).unwrap();
    assert_eq!(source.comments[0].alternatives.len(), 2);
    let again =
        jade_xlsx_writer::notes::upsert_comment(&out, CellRef::new(0, 0), "author", "again")
            .unwrap();
    assert_eq!(
        jade_xlsx_reader::comments::parse_comments(&again).unwrap()[0].text,
        "again"
    );
}
