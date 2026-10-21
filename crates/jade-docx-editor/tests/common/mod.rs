//! Word packages built from a body's XML, for the drawing-object tests.

#![allow(dead_code)]

use std::io::{Read, Write};

use jade_docx_editor::commands::{Command, CommandContext, CommandExecutor};
use jade_docx_editor::word_document::WordDocument;
use jade_docx_layout::selection::{SelectionRange, TextPosition};

/// The namespaces Word declares on `w:document`.
pub const NAMESPACES: &str = concat!(
    r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" "#,
    r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
    r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" "#,
    r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
    r#"xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" "#,
    r#"xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" "#,
    r#"xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup" "#,
    r#"xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" "#,
    r#"xmlns:a14="http://schemas.microsoft.com/office/drawing/2010/main" "#,
    r#"xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" "#,
    r#"xmlns:wp14="http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing""#,
);

/// A 2×1 PNG: one red pixel, one blue.
pub fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&[255, 0, 0, 255, 0, 0, 255, 255])
            .unwrap();
    }
    bytes
}

/// A package whose body is `body`, with `media` (name, bytes) under
/// `word/media/` related as `rId<n>` in order from `rId10`.
pub fn package(body: &str, media: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NAMESPACES}><w:body>{body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    );
    let mut relationships = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (index, (name, _)) in media.iter().enumerate() {
        relationships.push_str(&format!(
            r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/{name}"/>"#,
            index + 10
        ));
    }
    relationships.push_str("</Relationships>");
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        let mut file = |name: &str, bytes: &[u8]| {
            zip.start_file(name, options).unwrap();
            zip.write_all(bytes).unwrap();
        };
        file(
            "[Content_Types].xml",
            br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="svg" ContentType="image/svg+xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        );
        file(
            "_rels/.rels",
            br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        );
        file("word/document.xml", document.as_bytes());
        file("word/_rels/document.xml.rels", relationships.as_bytes());
        for (name, bytes) in media {
            file(&format!("word/media/{name}"), bytes);
        }
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

pub fn load(bytes: &[u8]) -> WordDocument {
    futures::executor::block_on(WordDocument::load(bytes, &Default::default())).expect("load")
}

/// A part of a saved package, as text.
pub fn part(bytes: &[u8], name: &str) -> String {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut text = String::new();
    archive
        .by_name(name)
        .unwrap_or_else(|_| panic!("{name} is in the package"))
        .read_to_string(&mut text)
        .unwrap();
    text
}

pub fn document_xml(document: &WordDocument) -> String {
    part(
        &document.export_docx().expect("export"),
        "word/document.xml",
    )
}

/// Runs commands through one executor, as a session does.
pub struct Session {
    pub document: WordDocument,
    pub executor: CommandExecutor,
    pub selection: Option<SelectionRange>,
}

impl Session {
    pub fn new(document: WordDocument) -> Self {
        Self {
            document,
            executor: CommandExecutor::new(),
            selection: Some(SelectionRange::collapsed(TextPosition::new(0, 0))),
        }
    }

    pub fn open(body: &str, media: &[(&str, Vec<u8>)]) -> Self {
        Self::new(load(&package(body, media)))
    }

    pub fn run(&mut self, command: Box<dyn Command>) {
        self.try_run(command).expect("command");
    }

    pub fn try_run(&mut self, command: Box<dyn Command>) -> Result<(), String> {
        let mut ctx = CommandContext::new(&mut self.document, &mut self.selection);
        self.executor
            .execute(command, &mut ctx)
            .map_err(|error| format!("{error:?}"))
    }

    pub fn undo(&mut self) {
        let mut ctx = CommandContext::new(&mut self.document, &mut self.selection);
        self.executor.undo(&mut ctx).expect("undo");
    }

    pub fn redo(&mut self) {
        let mut ctx = CommandContext::new(&mut self.document, &mut self.selection);
        self.executor.redo(&mut ctx).expect("redo");
    }

    pub fn xml(&self) -> String {
        document_xml(&self.document)
    }

    /// Saves and opens the result, as a later session would.
    pub fn reopened(&self) -> WordDocument {
        load(&self.document.export_docx().expect("export"))
    }
}

/// A floating picture of `rId10`, with `sp_pr_extra` after its geometry and
/// `blip_extra` inside its `a:blip`.
pub fn floating_picture(name: &str, sp_pr_extra: &str, blip_extra: &str) -> String {
    format!(
        r#"<w:r><w:drawing><wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" relativeHeight="251659264" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="952500" cy="476250"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapSquare wrapText="bothSides"/><wp:docPr id="1" name="{name}"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="1" name="{name}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rId10">{blip_extra}</a:blip><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="952500" cy="476250"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom>{sp_pr_extra}</pic:spPr></pic:pic></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>"#
    )
}
