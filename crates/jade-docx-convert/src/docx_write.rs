//! A flow document as a `.docx` package: what every import ends in, so the
//! editor opens a converted file as it opens any Word document.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::flow::*;
use crate::xml::escape;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NAMESPACES: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture""#;
const RELATIONSHIP: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The `.docx` bytes of `document`.
pub fn write_docx(document: &FlowDocument) -> Result<Vec<u8>, String> {
    let mut writer = DocxWriter::default();
    let mut body = String::new();
    let sections: Vec<&FlowSection> = if document.sections.is_empty() {
        Vec::new()
    } else {
        document.sections.iter().collect()
    };
    let default_section = FlowSection::default();
    let sections = if sections.is_empty() {
        vec![&default_section]
    } else {
        sections
    };
    let last = sections.len() - 1;
    for (index, section) in sections.iter().enumerate() {
        let header = (!section.header.is_empty())
            .then(|| writer.story_part("header", &section.header, section.page.text_width()));
        let footer = (!section.footer.is_empty())
            .then(|| writer.story_part("footer", &section.footer, section.page.text_width()));
        let properties = section_properties(&section.page, header.as_deref(), footer.as_deref());
        let mut blocks = String::new();
        writer.blocks(
            &section.blocks,
            section.page.text_width(),
            &mut blocks,
            "word/document.xml",
        );
        if index == last {
            if section.blocks.is_empty() {
                blocks.push_str("<w:p/>");
            }
            body.push_str(&blocks);
            body.push_str(&properties);
        } else {
            body.push_str(&blocks);
            // A section ends at a paragraph carrying its properties.
            let _ = write!(body, "<w:p><w:pPr>{properties}</w:pPr></w:p>");
        }
    }
    let document_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document {NAMESPACES}><w:body>{body}</w:body></w:document>"#
    );
    writer.package(document, document_xml)
}

fn section_properties(page: &PageSetup, header: Option<&str>, footer: Option<&str>) -> String {
    let mut out = String::from("<w:sectPr>");
    if let Some(id) = header {
        let _ = write!(out, r#"<w:headerReference w:type="default" r:id="{id}"/>"#);
    }
    if let Some(id) = footer {
        let _ = write!(out, r#"<w:footerReference w:type="default" r:id="{id}"/>"#);
    }
    let orientation = if page.landscape() {
        r#" w:orient="landscape""#
    } else {
        ""
    };
    let _ = write!(
        out,
        r#"<w:pgSz w:w="{}" w:h="{}"{orientation}/><w:pgMar w:top="{}" w:right="{}" w:bottom="{}" w:left="{}" w:header="{}" w:footer="{}" w:gutter="0"/><w:cols w:space="425"/></w:sectPr>"#,
        page.width,
        page.height,
        page.top,
        page.right,
        page.bottom,
        page.left,
        page.header,
        page.footer
    );
    out
}

#[derive(Default)]
struct DocxWriter {
    /// Relationships of word/document.xml beyond the fixed ones: id → (type, target, external).
    relationships: Vec<(String, String, String, bool)>,
    /// Parts besides the main ones: name → bytes.
    parts: BTreeMap<String, Vec<u8>>,
    /// Relationships of the header and footer parts, by part.
    story_relationships: BTreeMap<String, Vec<(String, String, String, bool)>>,
    overrides: Vec<(String, String)>,
    images: usize,
    stories: usize,
    /// The numbering instances: (ordered, start of a new list).
    lists: Vec<bool>,
    /// The numbered list being continued, while one is.
    open_list: Option<usize>,
    links: BTreeMap<String, String>,
    drawing_id: usize,
}

impl DocxWriter {
    fn relationship_for(&mut self, part: &str, kind: &str, target: &str, external: bool) -> String {
        let list = if part == "word/document.xml" {
            &mut self.relationships
        } else {
            self.story_relationships
                .entry(part.to_string())
                .or_default()
        };
        if let Some((id, ..)) = list
            .iter()
            .find(|(_, k, t, e)| k == kind && t == target && *e == external)
        {
            return id.clone();
        }
        // rId1–rId9 are the fixed parts'.
        let id = format!("rId{}", list.len() + 10);
        list.push((id.clone(), kind.to_string(), target.to_string(), external));
        id
    }

    fn story_part(&mut self, kind: &str, blocks: &[Block], width: u32) -> String {
        self.stories += 1;
        let name = format!("{kind}{}.xml", self.stories);
        let part = format!("word/{name}");
        let mut body = String::new();
        self.blocks(blocks, width, &mut body, &part);
        let root = if kind == "header" { "hdr" } else { "ftr" };
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:{root} {NAMESPACES}>{body}</w:{root}>"#
        );
        self.parts.insert(part.clone(), xml.into_bytes());
        self.overrides.push((
            format!("/{part}"),
            format!("application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml"),
        ));
        self.relationship_for(
            "word/document.xml",
            &format!("{RELATIONSHIP}/{kind}"),
            &name,
            false,
        )
    }

    fn blocks(&mut self, blocks: &[Block], width: u32, out: &mut String, part: &str) {
        for (index, block) in blocks.iter().enumerate() {
            match block {
                Block::Paragraph(paragraph) => self.paragraph(paragraph, width, out, part),
                Block::Table(table) => {
                    self.open_list = None;
                    self.table(table, width, out, part);
                    // Two tables in a row would merge into one: a paragraph
                    // keeps them apart, as Word does.
                    if matches!(blocks.get(index + 1), Some(Block::Table(_)) | None) {
                        out.push_str("<w:p/>");
                    }
                }
            }
        }
    }

    fn paragraph(&mut self, paragraph: &Paragraph, width: u32, out: &mut String, part: &str) {
        out.push_str("<w:p>");
        let mut properties = String::new();
        match &paragraph.kind {
            ParagraphKind::Normal => {}
            ParagraphKind::Heading(level) => {
                let _ = write!(
                    properties,
                    r#"<w:pStyle w:val="Heading{}"/>"#,
                    level.clamp(&1, &9)
                );
            }
            ParagraphKind::Title => properties.push_str(r#"<w:pStyle w:val="Title"/>"#),
            ParagraphKind::Preformatted => {
                properties.push_str(r#"<w:pStyle w:val="HTMLPreformatted"/>"#)
            }
            ParagraphKind::Quote => properties.push_str(r#"<w:pStyle w:val="Quote"/>"#),
        }
        if paragraph.keep_with_next {
            properties.push_str("<w:keepNext/>");
        }
        if paragraph.page_break_before {
            properties.push_str("<w:pageBreakBefore/>");
        }
        match paragraph.list {
            Some(item) => {
                let num = if item.ordered {
                    match self.open_list {
                        Some(num) => num,
                        _ => {
                            self.lists.push(true);
                            let num = self.lists.len();
                            self.open_list = Some(num);
                            num
                        }
                    }
                } else {
                    // Bullets all share one instance.
                    if !self.lists.contains(&false) {
                        self.lists.push(false);
                    }
                    self.lists.iter().position(|ordered| !ordered).unwrap() + 1
                };
                let _ = write!(
                    properties,
                    r#"<w:numPr><w:ilvl w:val="{}"/><w:numId w:val="{num}"/></w:numPr>"#,
                    item.level.min(8)
                );
            }
            None => self.open_list = None,
        }
        if let Some(shading) = &paragraph.shading {
            let _ = write!(
                properties,
                r#"<w:shd w:val="clear" w:color="auto" w:fill="{shading}"/>"#
            );
        }
        let spacing = {
            let mut spacing = String::new();
            if let Some(before) = paragraph.space_before {
                let _ = write!(spacing, r#" w:before="{before}""#);
            }
            if let Some(after) = paragraph.space_after {
                let _ = write!(spacing, r#" w:after="{after}""#);
            }
            match paragraph.line_spacing {
                Some(LineSpacing::Multiple(line)) => {
                    let _ = write!(spacing, r#" w:line="{line}" w:lineRule="auto""#);
                }
                Some(LineSpacing::Exact(line)) => {
                    let _ = write!(spacing, r#" w:line="{line}" w:lineRule="exact""#);
                }
                Some(LineSpacing::AtLeast(line)) => {
                    let _ = write!(spacing, r#" w:line="{line}" w:lineRule="atLeast""#);
                }
                None => {}
            }
            spacing
        };
        if !spacing.is_empty() {
            let _ = write!(properties, "<w:spacing{spacing}/>");
        }
        if paragraph.indent_left != 0
            || paragraph.indent_right != 0
            || paragraph.indent_first_line != 0
        {
            let mut indent = String::from("<w:ind");
            if paragraph.indent_left != 0 {
                let _ = write!(indent, r#" w:left="{}""#, paragraph.indent_left);
            }
            if paragraph.indent_right != 0 {
                let _ = write!(indent, r#" w:right="{}""#, paragraph.indent_right);
            }
            if paragraph.indent_first_line > 0 {
                let _ = write!(indent, r#" w:firstLine="{}""#, paragraph.indent_first_line);
            } else if paragraph.indent_first_line < 0 {
                let _ = write!(indent, r#" w:hanging="{}""#, -paragraph.indent_first_line);
            }
            indent.push_str("/>");
            properties.push_str(&indent);
        }
        if let Some(alignment) = paragraph.alignment {
            let value = match alignment {
                Alignment::Left => "left",
                Alignment::Center => "center",
                Alignment::Right => "right",
                Alignment::Justify => "both",
            };
            let _ = write!(properties, r#"<w:jc w:val="{value}"/>"#);
        }
        if !properties.is_empty() {
            let _ = write!(out, "<w:pPr>{properties}</w:pPr>");
        }
        let mut open_link: Option<String> = None;
        for inline in &paragraph.inlines {
            let link = match inline {
                Inline::Text(_, format) => format.link.clone(),
                _ => None,
            };
            if link != open_link {
                if open_link.is_some() {
                    out.push_str("</w:hyperlink>");
                }
                if let Some(target) = &link {
                    if let Some(anchor) = target.strip_prefix('#') {
                        let _ = write!(
                            out,
                            r#"<w:hyperlink w:anchor="{}" w:history="1">"#,
                            escape(anchor)
                        );
                    } else {
                        let id = self.link(target, part);
                        let _ = write!(out, r#"<w:hyperlink r:id="{id}" w:history="1">"#);
                    }
                }
                open_link = link;
            }
            self.inline(inline, width, out, part);
        }
        if open_link.is_some() {
            out.push_str("</w:hyperlink>");
        }
        out.push_str("</w:p>");
    }

    fn link(&mut self, target: &str, part: &str) -> String {
        let key = format!("{part}\u{0}{target}");
        if let Some(id) = self.links.get(&key) {
            return id.clone();
        }
        let id = self.relationship_for(part, &format!("{RELATIONSHIP}/hyperlink"), target, true);
        self.links.insert(key, id.clone());
        id
    }

    fn inline(&mut self, inline: &Inline, width: u32, out: &mut String, part: &str) {
        match inline {
            Inline::Text(text, format) => {
                let properties = run_properties(format);
                // Tabs and newlines inside text become their own elements.
                let mut pieces = String::new();
                let mut current = String::new();
                let flush = |current: &mut String, pieces: &mut String| {
                    if !current.is_empty() {
                        let _ = write!(
                            pieces,
                            r#"<w:t xml:space="preserve">{}</w:t>"#,
                            escape(current)
                        );
                        current.clear();
                    }
                };
                for ch in text.chars() {
                    match ch {
                        '\t' => {
                            flush(&mut current, &mut pieces);
                            pieces.push_str("<w:tab/>");
                        }
                        '\n' | '\u{b}' => {
                            flush(&mut current, &mut pieces);
                            pieces.push_str("<w:br/>");
                        }
                        '\r' => {}
                        '\u{c}' => {
                            flush(&mut current, &mut pieces);
                            pieces.push_str(r#"<w:br w:type="page"/>"#);
                        }
                        _ => current.push(ch),
                    }
                }
                flush(&mut current, &mut pieces);
                let _ = write!(out, "<w:r>{properties}{pieces}</w:r>");
            }
            Inline::Tab => out.push_str("<w:r><w:tab/></w:r>"),
            Inline::LineBreak => out.push_str("<w:r><w:br/></w:r>"),
            Inline::PageBreak => out.push_str(r#"<w:r><w:br w:type="page"/></w:r>"#),
            Inline::Image(image) => self.image(image, width, out, part),
        }
    }

    fn image(&mut self, image: &Image, width: u32, out: &mut String, part: &str) {
        self.images += 1;
        let number = self.images;
        let name = format!("media/image{number}.{}", image.format.extension());
        self.parts
            .insert(format!("word/{name}"), image.data.clone());
        let id = self.relationship_for(part, &format!("{RELATIONSHIP}/image"), &name, false);
        let mut image = image.clone();
        // Never wider than the text: Word shrinks a picture that would be.
        image.fit_width(width as u64 * EMU_PER_TWIP);
        self.drawing_id += 1;
        let description = image
            .description
            .as_deref()
            .map(|text| format!(r#" descr="{}""#, escape(text)))
            .unwrap_or_default();
        let _ = write!(
            out,
            r#"<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{w}" cy="{h}"/><wp:docPr id="{doc}" name="Picture {doc}"{description}/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{doc}" name="image{number}.{ext}"{description}/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="{id}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{w}" cy="{h}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#,
            w = image.width,
            h = image.height,
            doc = self.drawing_id,
            ext = image.format.extension(),
        );
    }

    fn table(&mut self, table: &Table, width: u32, out: &mut String, part: &str) {
        let grid = table.grid_width().max(1);
        let columns = if table.columns.len() == grid {
            table.columns.clone()
        } else {
            Table::even_columns(grid, width)
        };
        out.push_str("<w:tbl><w:tblPr>");
        if table.borders {
            out.push_str(r#"<w:tblStyle w:val="TableGrid"/>"#);
        }
        out.push_str(r#"<w:tblW w:w="0" w:type="auto"/><w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" w:lastColumn="0" w:noHBand="0" w:noVBand="1"/></w:tblPr><w:tblGrid>"#);
        for column in &columns {
            let _ = write!(out, r#"<w:gridCol w:w="{column}"/>"#);
        }
        out.push_str("</w:tblGrid>");
        for row in &table.rows {
            out.push_str("<w:tr>");
            if row.header {
                out.push_str("<w:trPr><w:tblHeader/></w:trPr>");
            }
            let mut column = 0usize;
            for cell in &row.cells {
                let span = cell.span.max(1) as usize;
                let cell_width: u32 = columns.iter().skip(column).take(span).sum();
                column += span;
                let _ = write!(
                    out,
                    r#"<w:tc><w:tcPr><w:tcW w:w="{cell_width}" w:type="dxa"/>"#
                );
                if span > 1 {
                    let _ = write!(out, r#"<w:gridSpan w:val="{span}"/>"#);
                }
                match cell.vertical_merge {
                    VerticalMerge::None => {}
                    VerticalMerge::Restart => out.push_str(r#"<w:vMerge w:val="restart"/>"#),
                    VerticalMerge::Continue => out.push_str("<w:vMerge/>"),
                }
                if let Some(fill) = &cell.shading {
                    let _ = write!(
                        out,
                        r#"<w:shd w:val="clear" w:color="auto" w:fill="{fill}"/>"#
                    );
                }
                out.push_str("</w:tcPr>");
                let mut blocks = String::new();
                self.blocks(&cell.blocks, cell_width.max(360), &mut blocks, part);
                // A cell has to end in a paragraph.
                if !matches!(cell.blocks.last(), Some(Block::Paragraph(_)))
                    && !blocks.ends_with("<w:p/>")
                {
                    blocks.push_str("<w:p/>");
                }
                out.push_str(&blocks);
                out.push_str("</w:tc>");
            }
            out.push_str("</w:tr>");
        }
        out.push_str("</w:tbl>");
        self.open_list = None;
    }

    fn package(mut self, document: &FlowDocument, document_xml: String) -> Result<Vec<u8>, String> {
        let mut package =
            jade_opc::OpcPackage::open(EMPTY_ZIP).map_err(|error| error.to_string())?;
        let mut types = String::from(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>"#,
        );
        for format in [
            ImageFormat::Png,
            ImageFormat::Jpeg,
            ImageFormat::Gif,
            ImageFormat::Bmp,
            ImageFormat::Tiff,
            ImageFormat::Emf,
            ImageFormat::Wmf,
        ] {
            let _ = write!(
                types,
                r#"<Default Extension="{}" ContentType="{}"/>"#,
                format.extension(),
                format.mime()
            );
        }
        for (name, kind) in [
            (
                "/word/document.xml",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
            ),
            (
                "/word/styles.xml",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml",
            ),
            (
                "/word/settings.xml",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml",
            ),
            (
                "/word/numbering.xml",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml",
            ),
            (
                "/docProps/core.xml",
                "application/vnd.openxmlformats-package.core-properties+xml",
            ),
            (
                "/docProps/app.xml",
                "application/vnd.openxmlformats-officedocument.extended-properties+xml",
            ),
        ] {
            let _ = write!(
                types,
                r#"<Override PartName="{name}" ContentType="{kind}"/>"#
            );
        }
        for (name, kind) in &self.overrides {
            let _ = write!(
                types,
                r#"<Override PartName="{name}" ContentType="{kind}"/>"#
            );
        }
        types.push_str("</Types>");
        package.replace("[Content_Types].xml", types.into_bytes());
        package.replace(
            "_rels/.rels",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RELATIONSHIP}/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId3" Type="{RELATIONSHIP}/extended-properties" Target="docProps/app.xml"/></Relationships>"#
            )
            .into_bytes(),
        );
        package.replace("word/document.xml", document_xml.into_bytes());
        package.replace("word/styles.xml", styles(document).into_bytes());
        package.replace("word/settings.xml", SETTINGS.as_bytes().to_vec());
        package.replace("word/numbering.xml", numbering(&self.lists).into_bytes());
        let mut relationships = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{RELATIONSHIP}/styles" Target="styles.xml"/><Relationship Id="rId2" Type="{RELATIONSHIP}/settings" Target="settings.xml"/><Relationship Id="rId3" Type="{RELATIONSHIP}/numbering" Target="numbering.xml"/>"#
        );
        push_relationships(&mut relationships, &self.relationships);
        relationships.push_str("</Relationships>");
        package.replace("word/_rels/document.xml.rels", relationships.into_bytes());
        for (part, list) in std::mem::take(&mut self.story_relationships) {
            let mut xml = String::from(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
            );
            push_relationships(&mut xml, &list);
            xml.push_str("</Relationships>");
            let name = part.trim_start_matches("word/");
            package.replace(&format!("word/_rels/{name}.rels"), xml.into_bytes());
        }
        for (name, bytes) in std::mem::take(&mut self.parts) {
            package.replace(&name, bytes);
        }
        package.replace(
            "docProps/core.xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>{}</dc:title><dc:creator>{}</dc:creator></cp:coreProperties>"#,
                escape(document.title.as_deref().unwrap_or("")),
                escape(document.author.as_deref().unwrap_or("")),
            )
            .into_bytes(),
        );
        package.replace(
            "docProps/app.xml",
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>jade office</Application></Properties>"#
                .to_vec(),
        );
        package.write().map_err(|error| error.to_string())
    }
}

fn push_relationships(out: &mut String, list: &[(String, String, String, bool)]) {
    for (id, kind, target, external) in list {
        let mode = if *external {
            r#" TargetMode="External""#
        } else {
            ""
        };
        let _ = write!(
            out,
            r#"<Relationship Id="{id}" Type="{kind}" Target="{}"{mode}/>"#,
            escape(target)
        );
    }
}

/// `w:rPr` for a run's formatting, or nothing for a plain run.
pub(crate) fn run_properties(format: &CharFormat) -> String {
    let mut out = String::new();
    if format.link.is_some() {
        out.push_str(r#"<w:rStyle w:val="Hyperlink"/>"#);
    }
    if format.font.is_some() || format.east_asian_font.is_some() {
        out.push_str("<w:rFonts");
        if let Some(font) = &format.font {
            let font = escape(font);
            let _ = write!(out, r#" w:ascii="{font}" w:hAnsi="{font}" w:cs="{font}""#);
        }
        if let Some(font) = format.east_asian_font.as_ref().or(format.font.as_ref()) {
            let _ = write!(out, r#" w:eastAsia="{}""#, escape(font));
        }
        out.push_str("/>");
    }
    if format.bold {
        out.push_str("<w:b/><w:bCs/>");
    }
    if format.italic {
        out.push_str("<w:i/><w:iCs/>");
    }
    if format.all_caps {
        out.push_str("<w:caps/>");
    }
    if format.small_caps {
        out.push_str("<w:smallCaps/>");
    }
    if format.strike {
        out.push_str("<w:strike/>");
    }
    if format.hidden {
        out.push_str("<w:vanish/>");
    }
    if let Some(color) = &format.color {
        let _ = write!(out, r#"<w:color w:val="{color}"/>"#);
    }
    if let Some(size) = format.size {
        let _ = write!(out, r#"<w:sz w:val="{size}"/><w:szCs w:val="{size}"/>"#);
    }
    if let Some(highlight) = &format.highlight {
        let _ = write!(out, r#"<w:highlight w:val="{highlight}"/>"#);
    }
    if format.underline {
        out.push_str(r#"<w:u w:val="single"/>"#);
    }
    if let Some(fill) = &format.shading {
        let _ = write!(
            out,
            r#"<w:shd w:val="clear" w:color="auto" w:fill="{fill}"/>"#
        );
    }
    if format.superscript {
        out.push_str(r#"<w:vertAlign w:val="superscript"/>"#);
    } else if format.subscript {
        out.push_str(r#"<w:vertAlign w:val="subscript"/>"#);
    }
    if out.is_empty() {
        out
    } else {
        format!("<w:rPr>{out}</w:rPr>")
    }
}

const SETTINGS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:zoom w:percent="100"/><w:defaultTabStop w:val="720"/><w:characterSpacingControl w:val="doNotCompress"/><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>"#;

fn styles(document: &FlowDocument) -> String {
    let font = document
        .default_font
        .as_deref()
        .map(escape)
        .unwrap_or_else(|| "Times New Roman".to_string());
    let size = document.default_size.unwrap_or(21);
    let language = document.language.as_deref().unwrap_or("en-US");
    let east_asian = "ja-JP";
    let mut headings = String::new();
    let heading_sizes = [32u32, 28, 26, 24, 22, 22, 21, 21, 21];
    for level in 1..=9u32 {
        let size = heading_sizes[level as usize - 1];
        let _ = write!(
            headings,
            r#"<w:style w:type="paragraph" w:styleId="Heading{level}"><w:name w:val="heading {level}"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="9"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="240" w:after="60"/><w:outlineLvl w:val="{}"/></w:pPr><w:rPr><w:b/><w:bCs/><w:sz w:val="{size}"/><w:szCs w:val="{size}"/></w:rPr></w:style>"#,
            level - 1
        );
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="{W}" xmlns:r="{R}"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="{font}" w:eastAsia="{font}" w:hAnsi="{font}" w:cs="{font}"/><w:sz w:val="{size}"/><w:szCs w:val="{size}"/><w:lang w:val="{language}" w:eastAsia="{east_asian}" w:bidi="ar-SA"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr/></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>{headings}<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:before="240" w:after="120"/><w:jc w:val="center"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="40"/><w:szCs w:val="40"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:ind w:left="864" w:right="864"/></w:pPr><w:rPr><w:i/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="HTMLPreformatted"><w:name w:val="HTML Preformatted"/><w:basedOn w:val="Normal"/><w:rPr><w:rFonts w:ascii="Courier New" w:hAnsi="Courier New" w:cs="Courier New"/><w:sz w:val="20"/><w:szCs w:val="20"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:ind w:left="720"/></w:pPr></w:style><w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont"><w:name w:val="Default Paragraph Font"/><w:uiPriority w:val="1"/><w:semiHidden/><w:unhideWhenUsed/></w:style><w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/><w:basedOn w:val="DefaultParagraphFont"/><w:uiPriority w:val="99"/><w:unhideWhenUsed/><w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr></w:style><w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style><w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:basedOn w:val="TableNormal"/><w:uiPriority w:val="39"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:left w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:right w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="auto"/></w:tblBorders></w:tblPr></w:style><w:style w:type="numbering" w:default="1" w:styleId="NoList"><w:name w:val="No List"/><w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/></w:style></w:styles>"#
    )
}

/// The bullet and numbered list definitions, and an instance for each
/// list the document has.
fn numbering(lists: &[bool]) -> String {
    let mut out = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering xmlns:w="{W}">"#
    );
    let bullets = ['●', '○', '■', '●', '○', '■', '●', '○', '■'];
    out.push_str(
        r#"<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="hybridMultilevel"/>"#,
    );
    for (level, bullet) in bullets.iter().enumerate() {
        let _ = write!(
            out,
            r#"<w:lvl w:ilvl="{level}"><w:start w:val="1"/><w:numFmt w:val="bullet"/><w:lvlText w:val="{bullet}"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="{}" w:hanging="360"/></w:pPr></w:lvl>"#,
            720 + 720 * level
        );
    }
    out.push_str("</w:abstractNum>");
    out.push_str(
        r#"<w:abstractNum w:abstractNumId="1"><w:multiLevelType w:val="hybridMultilevel"/>"#,
    );
    let formats = ["decimal", "lowerLetter", "lowerRoman"];
    for level in 0..9usize {
        let format = formats[level % 3];
        let _ = write!(
            out,
            r#"<w:lvl w:ilvl="{level}"><w:start w:val="1"/><w:numFmt w:val="{format}"/><w:lvlText w:val="%{}."/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="{}" w:hanging="360"/></w:pPr></w:lvl>"#,
            level + 1,
            720 + 720 * level
        );
    }
    out.push_str("</w:abstractNum>");
    for (index, ordered) in lists.iter().enumerate() {
        let num = index + 1;
        if *ordered {
            let _ = write!(
                out,
                r#"<w:num w:numId="{num}"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="1"/></w:lvlOverride></w:num>"#
            );
        } else {
            let _ = write!(
                out,
                r#"<w:num w:numId="{num}"><w:abstractNumId w:val="0"/></w:num>"#
            );
        }
    }
    out.push_str("</w:numbering>");
    out
}

/// A ZIP archive with nothing in it: just the end of its central directory.
const EMPTY_ZIP: &[u8] = &[
    0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];
