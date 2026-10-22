//! OpenDocument Text (`.odt`), in and out, as ODF 1.2 lays it out: a ZIP
//! with `mimetype` first and stored, `content.xml` for the text and its
//! automatic styles, `styles.xml` for the page and its header and footer,
//! `meta.xml`, and the pictures under `Pictures/`.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Read, Write};

use crate::flow::*;
use crate::xml::{escape, parse, Element, Node};

const MIMETYPE: &str = "application/vnd.oasis.opendocument.text";

// ---- writing ---------------------------------------------------------------

/// A document as `.odt` bytes.
pub fn write_odt(document: &FlowDocument) -> Result<Vec<u8>, String> {
    let mut writer = OdtWriter::default();
    let mut body = String::new();
    for (index, section) in document.sections.iter().enumerate() {
        // Each section after the first starts on a page of its own master.
        writer.section = index;
        writer.section_starts = index > 0;
        writer.blocks(&section.blocks, section.page.text_width(), &mut body);
    }
    let mut content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content {NS} office:version="1.2"><office:font-face-decls>{}</office:font-face-decls><office:automatic-styles>"#,
        writer.font_faces(document)
    );
    content.push_str(&writer.automatic);
    content.push_str(LIST_STYLES);
    content.push_str("</office:automatic-styles><office:body><office:text>");
    content.push_str(&body);
    content.push_str("</office:text></office:body></office:document-content>");

    let mut layouts = String::new();
    let mut masters = String::new();
    let mut story_styles = String::new();
    for (index, section) in document.sections.iter().enumerate() {
        let page = &section.page;
        let twips = |value: u32| format!("{:.3}in", value as f64 / 1440.0);
        let header_height = page.top.saturating_sub(page.header);
        let footer_height = page.bottom.saturating_sub(page.footer);
        let _ = write!(
            layouts,
            r#"<style:page-layout style:name="Layout{index}"><style:page-layout-properties fo:page-width="{}" fo:page-height="{}" style:print-orientation="{}" fo:margin-top="{}" fo:margin-bottom="{}" fo:margin-left="{}" fo:margin-right="{}"/><style:header-style><style:header-footer-properties fo:min-height="0in" fo:margin-bottom="{}"/></style:header-style><style:footer-style><style:header-footer-properties fo:min-height="0in" fo:margin-top="{}"/></style:footer-style></style:page-layout>"#,
            twips(page.width),
            twips(page.height),
            if page.landscape() {
                "landscape"
            } else {
                "portrait"
            },
            twips(if section.header.is_empty() {
                page.top
            } else {
                page.header
            }),
            twips(if section.footer.is_empty() {
                page.bottom
            } else {
                page.footer
            }),
            twips(page.left),
            twips(page.right),
            twips(header_height.max(1)),
            twips(footer_height.max(1)),
        );
        let mut header = String::new();
        writer.blocks(&section.header, page.text_width(), &mut header);
        let mut footer = String::new();
        writer.blocks(&section.footer, page.text_width(), &mut footer);
        let _ = write!(
            masters,
            r#"<style:master-page style:name="{}" style:page-layout-name="Layout{index}">"#,
            master_name(index)
        );
        if !section.header.is_empty() {
            let _ = write!(masters, "<style:header>{header}</style:header>");
        }
        if !section.footer.is_empty() {
            let _ = write!(masters, "<style:footer>{footer}</style:footer>");
        }
        masters.push_str("</style:master-page>");
    }
    // Styles the header and footer use are automatic styles of styles.xml.
    story_styles.push_str(&writer.automatic);
    let styles = format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<office:document-styles {NS} office:version="1.2"><office:font-face-decls>{fonts}</office:font-face-decls><office:styles><style:default-style style:family="paragraph"><style:text-properties style:font-name="{font}" fo:font-size="{size}pt" style:font-name-asian="{font}" style:font-size-asian="{size}pt"/></style:default-style><style:style style:name="Standard" style:family="paragraph" style:class="text"/>{headings}<style:style style:name="Title" style:family="paragraph" style:parent-style-name="Standard" style:class="chapter"><style:paragraph-properties fo:text-align="center" fo:margin-top="0.17in" fo:margin-bottom="0.08in"/><style:text-properties fo:font-size="20pt" fo:font-weight="bold" style:font-weight-asian="bold"/></style:style><style:style style:name="Quotations" style:family="paragraph" style:parent-style-name="Standard"><style:paragraph-properties fo:margin-left="0.6in" fo:margin-right="0.6in"/><style:text-properties fo:font-style="italic"/></style:style><style:style style:name="Preformatted_20_Text" style:display-name="Preformatted Text" style:family="paragraph" style:parent-style-name="Standard"><style:text-properties style:font-name="Courier New" fo:font-size="10pt"/></style:style><style:style style:name="Internet_20_link" style:display-name="Internet link" style:family="text"><style:text-properties fo:color="#0563c1" style:text-underline-style="solid" style:text-underline-width="auto" style:text-underline-color="font-color"/></style:style></office:styles><office:automatic-styles>{layouts}{story_styles}</office:automatic-styles><office:master-styles>{masters}</office:master-styles></office:document-styles>"##,
        fonts = writer.font_faces(document),
        font = escape(
            document
                .default_font
                .as_deref()
                .unwrap_or("Times New Roman")
        ),
        size = document.default_size.unwrap_or(21) as f64 / 2.0,
        headings = heading_styles(),
    );
    let meta = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-meta {NS} office:version="1.2"><office:meta><meta:generator>jade office</meta:generator>{}{}</office:meta></office:document-meta>"#,
        document
            .title
            .as_deref()
            .map(|title| format!("<dc:title>{}</dc:title>", escape(title)))
            .unwrap_or_default(),
        document
            .author
            .as_deref()
            .map(|author| format!("<dc:creator>{}</dc:creator>", escape(author)))
            .unwrap_or_default(),
    );
    let mut manifest = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2"><manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="application/vnd.oasis.opendocument.text"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="meta.xml" manifest:media-type="text/xml"/>"#,
    );
    for (name, image) in &writer.pictures {
        let _ = write!(
            manifest,
            r#"<manifest:file-entry manifest:full-path="{name}" manifest:media-type="{}"/>"#,
            image.format.mime()
        );
    }
    manifest.push_str("</manifest:manifest>");

    let mut buffer = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let deflated = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let error = |error: zip::result::ZipError| error.to_string();
        zip.start_file("mimetype", stored).map_err(error)?;
        zip.write_all(MIMETYPE.as_bytes())
            .map_err(|e| e.to_string())?;
        for (name, data) in [
            ("content.xml", content.as_bytes()),
            ("styles.xml", styles.as_bytes()),
            ("meta.xml", meta.as_bytes()),
            ("META-INF/manifest.xml", manifest.as_bytes()),
        ] {
            zip.start_file(name, deflated).map_err(error)?;
            zip.write_all(data).map_err(|e| e.to_string())?;
        }
        for (name, image) in &writer.pictures {
            zip.start_file(name.as_str(), stored).map_err(error)?;
            zip.write_all(&image.data).map_err(|e| e.to_string())?;
        }
        zip.finish().map_err(error)?;
    }
    Ok(buffer)
}

const NS: &str = r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0""#;

const LIST_STYLES: &str = r#"<text:list-style style:name="Bullets"><text:list-level-style-bullet text:level="1" text:bullet-char="●"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" fo:text-indent="-0.25in" fo:margin-left="0.5in"/></style:list-level-properties></text:list-level-style-bullet><text:list-level-style-bullet text:level="2" text:bullet-char="○"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" fo:text-indent="-0.25in" fo:margin-left="1in"/></style:list-level-properties></text:list-level-style-bullet><text:list-level-style-bullet text:level="3" text:bullet-char="■"><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" fo:text-indent="-0.25in" fo:margin-left="1.5in"/></style:list-level-properties></text:list-level-style-bullet></text:list-style><text:list-style style:name="Numbers"><text:list-level-style-number text:level="1" style:num-format="1" style:num-suffix="."><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" fo:text-indent="-0.25in" fo:margin-left="0.5in"/></style:list-level-properties></text:list-level-style-number><text:list-level-style-number text:level="2" style:num-format="a" style:num-suffix="."><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" fo:text-indent="-0.25in" fo:margin-left="1in"/></style:list-level-properties></text:list-level-style-number><text:list-level-style-number text:level="3" style:num-format="i" style:num-suffix="."><style:list-level-properties text:list-level-position-and-space-mode="label-alignment"><style:list-level-label-alignment text:label-followed-by="listtab" fo:text-indent="-0.25in" fo:margin-left="1.5in"/></style:list-level-properties></text:list-level-style-number></text:list-style>"#;

/// The first section's master is ODF's default page, `Standard`.
fn master_name(index: usize) -> String {
    if index == 0 {
        "Standard".to_string()
    } else {
        format!("Master{index}")
    }
}

fn heading_styles() -> String {
    let mut out = String::new();
    for level in 1..=9u32 {
        let size = [16.0, 14.0, 13.0, 12.0, 11.0, 11.0, 10.5, 10.5, 10.5][level as usize - 1];
        let _ = write!(
            out,
            r#"<style:style style:name="Heading_20_{level}" style:display-name="Heading {level}" style:family="paragraph" style:parent-style-name="Standard" style:next-style-name="Standard" style:default-outline-level="{level}" style:class="text"><style:paragraph-properties fo:margin-top="0.17in" fo:margin-bottom="0.04in" fo:keep-with-next="always"/><style:text-properties fo:font-size="{size}pt" fo:font-weight="bold" style:font-size-asian="{size}pt" style:font-weight-asian="bold"/></style:style>"#
        );
    }
    out
}

#[derive(Default)]
struct OdtWriter {
    automatic: String,
    paragraph_styles: HashMap<String, String>,
    text_styles: HashMap<String, String>,
    pictures: Vec<(String, Image)>,
    fonts: Vec<String>,
    section: usize,
    section_starts: bool,
    tables: usize,
}

fn length(twips: i64) -> String {
    format!("{:.4}in", twips as f64 / 1440.0)
}

impl OdtWriter {
    fn font_faces(&self, document: &FlowDocument) -> String {
        let mut names: Vec<&str> = self.fonts.iter().map(String::as_str).collect();
        names.push(
            document
                .default_font
                .as_deref()
                .unwrap_or("Times New Roman"),
        );
        names.push("Courier New");
        names.sort_unstable();
        names.dedup();
        names
            .iter()
            .map(|name| {
                let name = escape(name);
                format!(
                    r#"<style:font-face style:name="{name}" svg:font-family="&apos;{name}&apos;"/>"#
                )
            })
            .collect()
    }

    fn paragraph_style(&mut self, paragraph: &Paragraph, parent: &str) -> String {
        let mut properties = String::new();
        if let Some(alignment) = paragraph.alignment {
            let value = match alignment {
                Alignment::Left => "start",
                Alignment::Center => "center",
                Alignment::Right => "end",
                Alignment::Justify => "justify",
            };
            let _ = write!(properties, r#" fo:text-align="{value}""#);
        }
        if paragraph.list.is_none() {
            if paragraph.indent_left != 0 {
                let _ = write!(
                    properties,
                    r#" fo:margin-left="{}""#,
                    length(paragraph.indent_left as i64)
                );
            }
            if paragraph.indent_first_line != 0 {
                let _ = write!(
                    properties,
                    r#" fo:text-indent="{}""#,
                    length(paragraph.indent_first_line as i64)
                );
            }
        }
        if paragraph.indent_right != 0 {
            let _ = write!(
                properties,
                r#" fo:margin-right="{}""#,
                length(paragraph.indent_right as i64)
            );
        }
        if let Some(before) = paragraph.space_before {
            let _ = write!(properties, r#" fo:margin-top="{}""#, length(before as i64));
        }
        if let Some(after) = paragraph.space_after {
            let _ = write!(
                properties,
                r#" fo:margin-bottom="{}""#,
                length(after as i64)
            );
        }
        match paragraph.line_spacing {
            Some(LineSpacing::Multiple(line)) => {
                let _ = write!(properties, r#" fo:line-height="{}%""#, line * 100 / 240);
            }
            Some(LineSpacing::Exact(line)) => {
                let _ = write!(properties, r#" fo:line-height="{}""#, length(line as i64));
            }
            Some(LineSpacing::AtLeast(line)) => {
                let _ = write!(
                    properties,
                    r#" style:line-height-at-least="{}""#,
                    length(line as i64)
                );
            }
            None => {}
        }
        if paragraph.page_break_before {
            properties.push_str(r#" fo:break-before="page""#);
        }
        if paragraph.keep_with_next {
            properties.push_str(r#" fo:keep-with-next="always""#);
        }
        if let Some(shading) = &paragraph.shading {
            let _ = write!(properties, r##" fo:background-color="#{shading}""##);
        }
        let master = if self.section_starts {
            self.section_starts = false;
            format!(r#" style:master-page-name="{}""#, master_name(self.section))
        } else {
            String::new()
        };
        if properties.is_empty() && master.is_empty() {
            return parent.to_string();
        }
        let key = format!("{parent}|{master}|{properties}");
        if let Some(name) = self.paragraph_styles.get(&key) {
            return name.clone();
        }
        let name = format!("P{}", self.paragraph_styles.len() + 1);
        let _ = write!(
            self.automatic,
            r#"<style:style style:name="{name}" style:family="paragraph" style:parent-style-name="{parent}"{master}><style:paragraph-properties{properties}/></style:style>"#
        );
        self.paragraph_styles.insert(key, name.clone());
        name
    }

    fn text_style(&mut self, format: &CharFormat) -> Option<String> {
        let mut properties = String::new();
        if format.bold {
            properties.push_str(r#" fo:font-weight="bold" style:font-weight-asian="bold""#);
        }
        if format.italic {
            properties.push_str(r#" fo:font-style="italic" style:font-style-asian="italic""#);
        }
        if format.underline {
            properties.push_str(r#" style:text-underline-style="solid" style:text-underline-width="auto" style:text-underline-color="font-color""#);
        }
        if format.strike {
            properties.push_str(r#" style:text-line-through-style="solid""#);
        }
        if format.superscript {
            properties.push_str(r#" style:text-position="super 58%""#);
        } else if format.subscript {
            properties.push_str(r#" style:text-position="sub 58%""#);
        }
        if format.small_caps {
            properties.push_str(r#" fo:font-variant="small-caps""#);
        }
        if format.all_caps {
            properties.push_str(r#" fo:text-transform="uppercase""#);
        }
        if format.hidden {
            properties.push_str(r#" text:display="none""#);
        }
        if let Some(size) = format.size {
            let size = size as f64 / 2.0;
            let _ = write!(
                properties,
                r#" fo:font-size="{size}pt" style:font-size-asian="{size}pt""#
            );
        }
        if let Some(font) = &format.font {
            if !self.fonts.contains(font) {
                self.fonts.push(font.clone());
            }
            let _ = write!(properties, r#" style:font-name="{}""#, escape(font));
        }
        if let Some(font) = &format.east_asian_font {
            if !self.fonts.contains(font) {
                self.fonts.push(font.clone());
            }
            let _ = write!(properties, r#" style:font-name-asian="{}""#, escape(font));
        }
        if let Some(color) = &format.color {
            let _ = write!(properties, r##" fo:color="#{color}""##);
        }
        let background = format.shading.clone().or_else(|| {
            format
                .highlight
                .as_deref()
                .and_then(highlight_color)
                .map(str::to_string)
        });
        if let Some(background) = background {
            let _ = write!(properties, r##" fo:background-color="#{background}""##);
        }
        if properties.is_empty() {
            return None;
        }
        if let Some(name) = self.text_styles.get(&properties) {
            return Some(name.clone());
        }
        let name = format!("T{}", self.text_styles.len() + 1);
        let _ = write!(
            self.automatic,
            r#"<style:style style:name="{name}" style:family="text"><style:text-properties{properties}/></style:style>"#
        );
        self.text_styles.insert(properties, name.clone());
        Some(name)
    }

    fn blocks(&mut self, blocks: &[Block], width: u32, out: &mut String) {
        let mut index = 0;
        while index < blocks.len() {
            match &blocks[index] {
                Block::Paragraph(paragraph) if paragraph.list.is_some() => {
                    // A run of list paragraphs is one list, nested by level.
                    let start = index;
                    while matches!(blocks.get(index), Some(Block::Paragraph(p)) if p.list.is_some())
                    {
                        index += 1;
                    }
                    let items: Vec<&Paragraph> = blocks[start..index]
                        .iter()
                        .filter_map(|block| match block {
                            Block::Paragraph(paragraph) => Some(paragraph),
                            _ => None,
                        })
                        .collect();
                    self.list(&items, 0, width, out);
                    continue;
                }
                Block::Paragraph(paragraph) => self.paragraph(paragraph, width, out),
                Block::Table(table) => self.table(table, width, out),
            }
            index += 1;
        }
    }

    fn list(&mut self, items: &[&Paragraph], level: u8, width: u32, out: &mut String) {
        let ordered = items
            .first()
            .and_then(|item| item.list)
            .is_some_and(|item| item.ordered);
        let style = if ordered { "Numbers" } else { "Bullets" };
        if level == 0 {
            let _ = write!(out, r#"<text:list text:style-name="{style}">"#);
        } else {
            out.push_str("<text:list>");
        }
        let mut index = 0;
        while index < items.len() {
            let item_level = items[index].list.map(|item| item.level).unwrap_or(0);
            out.push_str("<text:list-item>");
            if item_level > level {
                let start = index;
                while index < items.len()
                    && items[index].list.map(|item| item.level).unwrap_or(0) > level
                {
                    index += 1;
                }
                self.list(&items[start..index], level + 1, width, out);
            } else {
                self.paragraph(items[index], width, out);
                index += 1;
                // Deeper items right after belong in this item.
                let start = index;
                while index < items.len()
                    && items[index].list.map(|item| item.level).unwrap_or(0) > level
                {
                    index += 1;
                }
                if index > start {
                    self.list(&items[start..index], level + 1, width, out);
                }
            }
            out.push_str("</text:list-item>");
        }
        out.push_str("</text:list>");
    }

    fn paragraph(&mut self, paragraph: &Paragraph, width: u32, out: &mut String) {
        let (element, parent, level) = match paragraph.kind {
            ParagraphKind::Heading(level) => (
                "text:h",
                format!("Heading_20_{}", level.clamp(1, 9)),
                Some(level.clamp(1, 9)),
            ),
            ParagraphKind::Title => ("text:p", "Title".to_string(), None),
            ParagraphKind::Quote => ("text:p", "Quotations".to_string(), None),
            ParagraphKind::Preformatted => ("text:p", "Preformatted_20_Text".to_string(), None),
            ParagraphKind::Normal => ("text:p", "Standard".to_string(), None),
        };
        let style = self.paragraph_style(paragraph, &parent);
        let _ = write!(out, r#"<{element} text:style-name="{style}""#);
        if let Some(level) = level {
            let _ = write!(out, r#" text:outline-level="{level}""#);
        }
        out.push('>');
        let mut open_link: Option<&str> = None;
        for inline in &paragraph.inlines {
            let link = match inline {
                Inline::Text(_, format) => format.link.as_deref(),
                _ => None,
            };
            if link != open_link {
                if open_link.is_some() {
                    out.push_str("</text:a>");
                }
                if let Some(target) = link {
                    let _ = write!(
                        out,
                        r#"<text:a xlink:type="simple" xlink:href="{}" text:style-name="Internet_20_link">"#,
                        escape(target)
                    );
                }
                open_link = link;
            }
            match inline {
                Inline::Text(text, format) => {
                    let style = self.text_style(format);
                    if let Some(style) = &style {
                        let _ = write!(out, r#"<text:span text:style-name="{style}">"#);
                    }
                    push_odf_text(text, out);
                    if style.is_some() {
                        out.push_str("</text:span>");
                    }
                }
                Inline::Tab => out.push_str("<text:tab/>"),
                Inline::LineBreak => out.push_str("<text:line-break/>"),
                Inline::PageBreak => out.push_str("<text:soft-page-break/>"),
                Inline::Image(image) => {
                    let mut image = image.clone();
                    image.fit_width(width as u64 * EMU_PER_TWIP);
                    let name = format!(
                        "Pictures/image{}.{}",
                        self.pictures.len() + 1,
                        image.format.extension()
                    );
                    let inches = |emu: u64| format!("{:.4}in", emu as f64 / 914400.0);
                    let _ = write!(
                        out,
                        r#"<draw:frame draw:name="Picture {}" text:anchor-type="as-char" svg:width="{}" svg:height="{}"><draw:image xlink:href="{name}" xlink:type="simple" xlink:show="embed" xlink:actuate="onLoad"/>{}</draw:frame>"#,
                        self.pictures.len() + 1,
                        inches(image.width),
                        inches(image.height),
                        image
                            .description
                            .as_deref()
                            .map(|text| format!("<svg:desc>{}</svg:desc>", escape(text)))
                            .unwrap_or_default(),
                    );
                    self.pictures.push((name, image));
                }
            }
        }
        if open_link.is_some() {
            out.push_str("</text:a>");
        }
        let _ = write!(out, "</{element}>");
    }

    fn table(&mut self, table: &Table, width: u32, out: &mut String) {
        self.tables += 1;
        let name = format!("Table{}", self.tables);
        let grid = table.grid_width().max(1);
        let columns = if table.columns.len() == grid {
            table.columns.clone()
        } else {
            Table::even_columns(grid, width)
        };
        let total: u32 = columns.iter().sum();
        let _ = write!(
            self.automatic,
            r#"<style:style style:name="{name}" style:family="table"><style:table-properties style:width="{}" table:align="left"/></style:style>"#,
            length(total as i64)
        );
        for (index, column) in columns.iter().enumerate() {
            let _ = write!(
                self.automatic,
                r#"<style:style style:name="{name}.C{index}" style:family="table-column"><style:table-column-properties style:column-width="{}"/></style:style>"#,
                length(*column as i64)
            );
        }
        let border = if table.borders {
            "0.5pt solid #000000"
        } else {
            "none"
        };
        let _ = write!(
            self.automatic,
            r#"<style:style style:name="{name}.Cell" style:family="table-cell"><style:table-cell-properties fo:padding="0.075in" fo:border="{border}"/></style:style>"#
        );
        let _ = write!(
            out,
            r#"<table:table table:name="{name}" table:style-name="{name}">"#
        );
        for index in 0..columns.len() {
            let _ = write!(
                out,
                r#"<table:table-column table:style-name="{name}.C{index}"/>"#
            );
        }
        // Vertically merged cells: a restart spans the continues below it.
        let mut row_spans: HashMap<(usize, usize), usize> = HashMap::new();
        for (row_index, row) in table.rows.iter().enumerate() {
            let mut column = 0;
            for cell in &row.cells {
                if cell.vertical_merge == VerticalMerge::Restart {
                    let mut span = 1;
                    for below in table.rows.iter().skip(row_index + 1) {
                        let continues = cell_at(below, column)
                            .is_some_and(|below| below.vertical_merge == VerticalMerge::Continue);
                        if !continues {
                            break;
                        }
                        span += 1;
                    }
                    row_spans.insert((row_index, column), span);
                }
                column += cell.span.max(1) as usize;
            }
        }
        let header_rows = table.rows.iter().take_while(|row| row.header).count();
        for (row_index, row) in table.rows.iter().enumerate() {
            if row_index == 0 && header_rows > 0 {
                out.push_str("<table:table-header-rows>");
            }
            out.push_str("<table:table-row>");
            let mut column = 0;
            for cell in &row.cells {
                let span = cell.span.max(1) as usize;
                let cell_width: u32 = columns.iter().skip(column).take(span).sum();
                if cell.vertical_merge == VerticalMerge::Continue {
                    out.push_str("<table:covered-table-cell/>");
                } else {
                    let style = match &cell.shading {
                        Some(fill) => {
                            let style = format!("{name}.Cell{fill}");
                            if !self.automatic.contains(&format!(r#"style:name="{style}""#)) {
                                let _ = write!(
                                    self.automatic,
                                    r##"<style:style style:name="{style}" style:family="table-cell"><style:table-cell-properties fo:padding="0.075in" fo:border="{border}" fo:background-color="#{fill}"/></style:style>"##
                                );
                            }
                            style
                        }
                        None => format!("{name}.Cell"),
                    };
                    let _ = write!(
                        out,
                        r#"<table:table-cell table:style-name="{style}" office:value-type="string""#
                    );
                    if span > 1 {
                        let _ = write!(out, r#" table:number-columns-spanned="{span}""#);
                    }
                    if let Some(rows) = row_spans.get(&(row_index, column)) {
                        let _ = write!(out, r#" table:number-rows-spanned="{rows}""#);
                    }
                    out.push('>');
                    if cell.blocks.is_empty() {
                        out.push_str(r#"<text:p text:style-name="Standard"/>"#);
                    }
                    self.blocks(&cell.blocks, cell_width, out);
                    out.push_str("</table:table-cell>");
                }
                for _ in 1..span {
                    out.push_str("<table:covered-table-cell/>");
                }
                column += span;
            }
            out.push_str("</table:table-row>");
            if row_index + 1 == header_rows {
                out.push_str("</table:table-header-rows>");
            }
        }
        out.push_str("</table:table>");
    }
}

fn cell_at(row: &Row, column: usize) -> Option<&Cell> {
    let mut at = 0;
    for cell in &row.cells {
        if at == column {
            return Some(cell);
        }
        at += cell.span.max(1) as usize;
        if at > column {
            return None;
        }
    }
    None
}

/// Text in ODF: runs of spaces as `text:s`, tabs and breaks as elements.
fn push_odf_text(text: &str, out: &mut String) {
    let mut spaces = 0usize;
    let flush = |spaces: &mut usize, out: &mut String| {
        if *spaces > 0 {
            out.push(' ');
            if *spaces > 1 {
                let _ = write!(out, r#"<text:s text:c="{}"/>"#, *spaces - 1);
            }
            *spaces = 0;
        }
    };
    for ch in text.chars() {
        match ch {
            ' ' => spaces += 1,
            '\t' => {
                flush(&mut spaces, out);
                out.push_str("<text:tab/>");
            }
            '\n' => {
                flush(&mut spaces, out);
                out.push_str("<text:line-break/>");
            }
            _ => {
                flush(&mut spaces, out);
                out.push_str(&escape(&ch.to_string()));
            }
        }
    }
    flush(&mut spaces, out);
}

// ---- reading ---------------------------------------------------------------

/// Reads an `.odt` (or a flat `.fodt`) document.
pub fn read_odt(bytes: &[u8]) -> Result<FlowDocument, String> {
    let (content, styles, meta, pictures) = if bytes.starts_with(b"PK") {
        let mut archive =
            zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let mut read = |name: &str| -> Option<Vec<u8>> {
            let mut file = archive.by_name(name).ok()?;
            let mut out = Vec::new();
            file.read_to_end(&mut out).ok()?;
            Some(out)
        };
        let content = read("content.xml").ok_or("the document has no content.xml")?;
        let styles = read("styles.xml");
        let meta = read("meta.xml");
        let mut pictures = HashMap::new();
        let names: Vec<String> = archive.file_names().map(str::to_string).collect();
        for name in names {
            if name.starts_with("Pictures/") || name.starts_with("media/") {
                if let Ok(mut file) = archive.by_name(&name) {
                    let mut out = Vec::new();
                    if file.read_to_end(&mut out).is_ok() {
                        pictures.insert(name, out);
                    }
                }
            }
        }
        (content, styles, meta, pictures)
    } else {
        // A flat ODT has everything in one document.
        (bytes.to_vec(), None, None, HashMap::new())
    };
    let content = parse(&String::from_utf8_lossy(&content))?;
    let styles = styles.and_then(|styles| parse(&String::from_utf8_lossy(&styles)).ok());
    let meta = meta.and_then(|meta| parse(&String::from_utf8_lossy(&meta)).ok());
    let mut reader = OdtReader {
        styles: HashMap::new(),
        list_styles: HashMap::new(),
        page_layouts: HashMap::new(),
        masters: HashMap::new(),
        pictures,
        default_text: TextProps::default(),
    };
    for root in styles.iter().chain(std::iter::once(&content)) {
        reader.read_styles(root);
    }
    let mut document = FlowDocument::default();
    let meta_root = meta.as_ref().or(Some(&content));
    if let Some(meta) = meta_root.and_then(|root| root.find("meta")) {
        document.title = meta
            .child("title")
            .map(Element::text)
            .filter(|t| !t.trim().is_empty());
        document.author = meta
            .child("creator")
            .or_else(|| meta.child("initial-creator"))
            .map(Element::text)
            .filter(|t| !t.trim().is_empty());
    }
    document.default_font = reader.default_text.font.clone();
    document.default_size = reader.default_text.size;
    let text = content
        .find("body")
        .and_then(|body| body.child("text"))
        .ok_or("the document has no text")?;
    // Sections follow master pages: a paragraph whose style names another
    // master starts a new one.
    let first_master = reader
        .first_master(text)
        .unwrap_or_else(|| "Standard".to_string());
    let mut section = reader.section_for(&first_master);
    let mut started = false;
    for element in text.elements() {
        if let Some(master) = reader.master_of(element) {
            if started && !section.blocks.is_empty() {
                document
                    .sections
                    .push(std::mem::replace(&mut section, reader.section_for(&master)));
            } else {
                let blocks = std::mem::take(&mut section.blocks);
                section = reader.section_for(&master);
                section.blocks = blocks;
            }
        }
        started = true;
        reader.block(element, &mut section.blocks, None);
    }
    document.sections.push(section);
    Ok(document)
}

#[derive(Clone, Debug, Default)]
struct TextProps {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    position: Option<i8>,
    small_caps: Option<bool>,
    caps: Option<bool>,
    hidden: Option<bool>,
    size: Option<u32>,
    font: Option<String>,
    east_asian_font: Option<String>,
    color: Option<String>,
    background: Option<String>,
}

impl TextProps {
    fn read(properties: &Element) -> Self {
        let mut out = Self::default();
        if let Some(weight) = properties.attr("fo:font-weight") {
            out.bold = Some(weight == "bold" || weight.parse::<u32>().is_ok_and(|w| w >= 600));
        }
        if let Some(style) = properties.attr("fo:font-style") {
            out.italic = Some(style == "italic" || style == "oblique");
        }
        if let Some(underline) = properties.attr("style:text-underline-style") {
            out.underline = Some(underline != "none");
        }
        if let Some(through) = properties.attr("style:text-line-through-style") {
            out.strike = Some(through != "none");
        }
        if let Some(position) = properties.attr("style:text-position") {
            let first = position.split_whitespace().next().unwrap_or("0");
            out.position = Some(
                if first == "super"
                    || first
                        .trim_end_matches('%')
                        .parse::<f64>()
                        .is_ok_and(|p| p > 0.0)
                {
                    1
                } else if first == "sub"
                    || first
                        .trim_end_matches('%')
                        .parse::<f64>()
                        .is_ok_and(|p| p < 0.0)
                {
                    -1
                } else {
                    0
                },
            );
        }
        if let Some(variant) = properties.attr("fo:font-variant") {
            out.small_caps = Some(variant == "small-caps");
        }
        if let Some(transform) = properties.attr("fo:text-transform") {
            out.caps = Some(transform == "uppercase");
        }
        if let Some(display) = properties.attr("text:display") {
            out.hidden = Some(display == "none");
        }
        if let Some(size) = properties.attr("fo:font-size").and_then(points) {
            out.size = Some((size * 2.0).round() as u32);
        }
        out.font = properties
            .attr("style:font-name")
            .or_else(|| properties.attr("fo:font-family"))
            .map(|font| font.trim_matches('\'').to_string());
        out.east_asian_font = properties
            .attr("style:font-name-asian")
            .map(|font| font.trim_matches('\'').to_string());
        out.color = properties.attr("fo:color").and_then(hex_color);
        out.background = properties
            .attr("fo:background-color")
            .filter(|color| *color != "transparent")
            .and_then(hex_color);
        out
    }

    fn merged(&self, over: &Self) -> Self {
        Self {
            bold: over.bold.or(self.bold),
            italic: over.italic.or(self.italic),
            underline: over.underline.or(self.underline),
            strike: over.strike.or(self.strike),
            position: over.position.or(self.position),
            small_caps: over.small_caps.or(self.small_caps),
            caps: over.caps.or(self.caps),
            hidden: over.hidden.or(self.hidden),
            size: over.size.or(self.size),
            font: over.font.clone().or_else(|| self.font.clone()),
            east_asian_font: over
                .east_asian_font
                .clone()
                .or_else(|| self.east_asian_font.clone()),
            color: over.color.clone().or_else(|| self.color.clone()),
            background: over.background.clone().or_else(|| self.background.clone()),
        }
    }

    fn format(&self, defaults: &TextProps) -> CharFormat {
        CharFormat {
            bold: self.bold.unwrap_or(false),
            italic: self.italic.unwrap_or(false),
            underline: self.underline.unwrap_or(false),
            strike: self.strike.unwrap_or(false),
            superscript: self.position == Some(1),
            subscript: self.position == Some(-1),
            small_caps: self.small_caps.unwrap_or(false),
            all_caps: self.caps.unwrap_or(false),
            hidden: self.hidden.unwrap_or(false),
            size: self.size.filter(|size| Some(*size) != defaults.size),
            font: self
                .font
                .clone()
                .filter(|font| Some(font) != defaults.font.as_ref()),
            east_asian_font: self
                .east_asian_font
                .clone()
                .filter(|font| Some(font) != defaults.east_asian_font.as_ref()),
            color: self.color.clone(),
            highlight: None,
            shading: self.background.clone(),
            link: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ParaProps {
    alignment: Option<Alignment>,
    left: Option<i32>,
    right: Option<i32>,
    first: Option<i32>,
    before: Option<u32>,
    after: Option<u32>,
    line: Option<LineSpacing>,
    break_before: Option<bool>,
    keep_with_next: Option<bool>,
    background: Option<String>,
}

impl ParaProps {
    fn read(properties: &Element) -> Self {
        let twips = |name: &str| properties.attr(name).and_then(length_twips);
        Self {
            alignment: properties.attr("fo:text-align").map(|align| match align {
                "center" => Alignment::Center,
                "end" | "right" => Alignment::Right,
                "justify" => Alignment::Justify,
                _ => Alignment::Left,
            }),
            left: twips("fo:margin-left").map(|v| v as i32),
            right: twips("fo:margin-right").map(|v| v as i32),
            first: twips("fo:text-indent").map(|v| v as i32),
            before: twips("fo:margin-top").map(|v| v.max(0) as u32),
            after: twips("fo:margin-bottom").map(|v| v.max(0) as u32),
            line: properties
                .attr("fo:line-height")
                .and_then(|value| {
                    if let Some(percent) = value.strip_suffix('%') {
                        percent
                            .parse::<f64>()
                            .ok()
                            .map(|p| LineSpacing::Multiple((p * 2.4).round() as u32))
                    } else {
                        length_twips(value).map(|v| LineSpacing::Exact(v.max(0) as u32))
                    }
                })
                .or_else(|| {
                    properties
                        .attr("style:line-height-at-least")
                        .and_then(length_twips)
                        .map(|v| LineSpacing::AtLeast(v.max(0) as u32))
                }),
            break_before: properties
                .attr("fo:break-before")
                .map(|value| value == "page"),
            keep_with_next: properties
                .attr("fo:keep-with-next")
                .map(|value| value == "always"),
            background: properties
                .attr("fo:background-color")
                .filter(|color| *color != "transparent")
                .and_then(hex_color),
        }
    }

    fn merged(&self, over: &Self) -> Self {
        Self {
            alignment: over.alignment.or(self.alignment),
            left: over.left.or(self.left),
            right: over.right.or(self.right),
            first: over.first.or(self.first),
            before: over.before.or(self.before),
            after: over.after.or(self.after),
            line: over.line.or(self.line),
            break_before: over.break_before.or(self.break_before),
            keep_with_next: over.keep_with_next.or(self.keep_with_next),
            background: over.background.clone().or_else(|| self.background.clone()),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct OdfStyle {
    name: String,
    column_width: Option<i64>,
    parent: Option<String>,
    display_name: Option<String>,
    outline: Option<u8>,
    master: Option<String>,
    text: TextProps,
    paragraph: ParaProps,
}

struct OdtReader {
    /// By family and name.
    styles: HashMap<(String, String), OdfStyle>,
    /// Each list style's levels: ordered or not.
    list_styles: HashMap<String, Vec<bool>>,
    page_layouts: HashMap<String, PageSetup>,
    /// Master page → (layout, header, footer).
    masters: HashMap<String, (String, Option<Element>, Option<Element>)>,
    pictures: HashMap<String, Vec<u8>>,
    default_text: TextProps,
}

impl OdtReader {
    fn read_styles(&mut self, root: &Element) {
        for container in root.elements() {
            match container.local() {
                "styles" | "automatic-styles" => {
                    for style in container.elements() {
                        match style.local() {
                            "style" => {
                                let (Some(name), Some(family)) =
                                    (style.attr("style:name"), style.attr("style:family"))
                                else {
                                    continue;
                                };
                                let text = style
                                    .child("text-properties")
                                    .map(TextProps::read)
                                    .unwrap_or_default();
                                let paragraph = style
                                    .child("paragraph-properties")
                                    .or_else(|| style.child("table-cell-properties"))
                                    .map(ParaProps::read)
                                    .unwrap_or_default();
                                self.styles.insert(
                                    (family.to_string(), name.to_string()),
                                    OdfStyle {
                                        name: name.to_string(),
                                        column_width: style
                                            .child("table-column-properties")
                                            .and_then(|properties| {
                                                properties.attr("style:column-width")
                                            })
                                            .and_then(length_twips),
                                        parent: style
                                            .attr("style:parent-style-name")
                                            .map(str::to_string),
                                        display_name: style
                                            .attr("style:display-name")
                                            .map(str::to_string),
                                        outline: style
                                            .attr("style:default-outline-level")
                                            .and_then(|l| l.parse().ok()),
                                        master: style
                                            .attr("style:master-page-name")
                                            .filter(|m| !m.is_empty())
                                            .map(str::to_string),
                                        text,
                                        paragraph,
                                    },
                                );
                            }
                            "default-style" if style.attr("style:family") == Some("paragraph") => {
                                if let Some(text) = style.child("text-properties") {
                                    self.default_text = TextProps::read(text);
                                }
                            }
                            "list-style" => {
                                let Some(name) = style.attr("style:name") else {
                                    continue;
                                };
                                let mut levels = vec![true; 10];
                                for level in style.elements() {
                                    let index = level
                                        .attr("text:level")
                                        .and_then(|l| l.parse::<usize>().ok())
                                        .unwrap_or(1)
                                        .clamp(1, 10)
                                        - 1;
                                    levels[index] = match level.local() {
                                        "list-level-style-bullet" | "list-level-style-image" => {
                                            false
                                        }
                                        "list-level-style-number" => level
                                            .attr("style:num-format")
                                            .is_some_and(|f| !f.is_empty()),
                                        _ => true,
                                    };
                                }
                                self.list_styles.insert(name.to_string(), levels);
                            }
                            "page-layout" => {
                                let Some(name) = style.attr("style:name") else {
                                    continue;
                                };
                                if let Some(properties) = style.child("page-layout-properties") {
                                    let twips = |key: &str| {
                                        properties
                                            .attr(key)
                                            .and_then(length_twips)
                                            .map(|v| v.max(0) as u32)
                                    };
                                    let defaults = PageSetup::default();
                                    let mut page = PageSetup {
                                        width: twips("fo:page-width").unwrap_or(defaults.width),
                                        height: twips("fo:page-height").unwrap_or(defaults.height),
                                        top: twips("fo:margin-top").unwrap_or(defaults.top),
                                        bottom: twips("fo:margin-bottom")
                                            .unwrap_or(defaults.bottom),
                                        left: twips("fo:margin-left").unwrap_or(defaults.left),
                                        right: twips("fo:margin-right").unwrap_or(defaults.right),
                                        header: 0,
                                        footer: 0,
                                    };
                                    // ODF's margin is to the header; Word's to the text.
                                    let header_height = style
                                        .child("header-style")
                                        .and_then(|h| h.child("header-footer-properties"))
                                        .map(|p| {
                                            p.attr("fo:min-height")
                                                .and_then(length_twips)
                                                .unwrap_or(0)
                                                + p.attr("fo:margin-bottom")
                                                    .and_then(length_twips)
                                                    .unwrap_or(0)
                                        });
                                    let footer_height = style
                                        .child("footer-style")
                                        .and_then(|h| h.child("header-footer-properties"))
                                        .map(|p| {
                                            p.attr("fo:min-height")
                                                .and_then(length_twips)
                                                .unwrap_or(0)
                                                + p.attr("fo:margin-top")
                                                    .and_then(length_twips)
                                                    .unwrap_or(0)
                                        });
                                    page.header = page.top;
                                    page.footer = page.bottom;
                                    if let Some(height) = header_height.filter(|h| *h > 0) {
                                        page.top += height.max(0) as u32;
                                    }
                                    if let Some(height) = footer_height.filter(|h| *h > 0) {
                                        page.bottom += height.max(0) as u32;
                                    }
                                    self.page_layouts.insert(name.to_string(), page);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "master-styles" => {
                    for master in container.children_named("master-page") {
                        let Some(name) = master.attr("style:name") else {
                            continue;
                        };
                        let layout = master
                            .attr("style:page-layout-name")
                            .unwrap_or("")
                            .to_string();
                        self.masters.insert(
                            name.to_string(),
                            (
                                layout,
                                master.child("header").cloned(),
                                master.child("footer").cloned(),
                            ),
                        );
                    }
                }
                _ => {}
            }
        }
    }

    fn style_chain(&self, family: &str, name: &str) -> Vec<&OdfStyle> {
        let mut out = Vec::new();
        let mut next = Some(name.to_string());
        while let Some(name) = next {
            let Some(style) = self.styles.get(&(family.to_string(), name)) else {
                break;
            };
            if out.len() > 16 {
                break;
            }
            out.push(style);
            next = style.parent.clone();
        }
        out
    }

    fn paragraph_style(&self, name: Option<&str>) -> (ParaProps, TextProps, Option<u8>, String) {
        let mut paragraph = ParaProps::default();
        let mut text = self.default_text.clone();
        let mut outline = None;
        let mut display = String::new();
        if let Some(name) = name {
            let chain = self.style_chain("paragraph", name);
            for style in chain.iter().rev() {
                paragraph = paragraph.merged(&style.paragraph);
                text = text.merged(&style.text);
                if style.outline.is_some() {
                    outline = style.outline;
                }
            }
            // The first style in the chain that is one of the kinds a
            // paragraph can be says which it is.
            display = chain
                .iter()
                .map(|style| {
                    style
                        .display_name
                        .clone()
                        .unwrap_or_else(|| style.name.replace("_20_", " "))
                })
                .find(|name| {
                    let lower = name.to_ascii_lowercase();
                    lower == "title"
                        || lower.starts_with("heading")
                        || lower.starts_with("quotation")
                        || lower.contains("preformatted")
                })
                .unwrap_or_default();
        }
        (paragraph, text, outline, display.replace("_20_", " "))
    }

    fn master_of(&self, element: &Element) -> Option<String> {
        let name = element.attr("text:style-name")?;
        self.style_chain("paragraph", name)
            .iter()
            .find_map(|style| style.master.clone())
    }

    fn first_master(&self, text: &Element) -> Option<String> {
        let first = text
            .elements()
            .find(|element| matches!(element.local(), "p" | "h" | "table" | "list"))?;
        self.master_of(first).or_else(|| {
            self.masters
                .keys()
                .find(|name| *name == "Standard")
                .cloned()
        })
    }

    fn section_for(&self, master: &str) -> FlowSection {
        let mut section = FlowSection::default();
        let Some((layout, header, footer)) = self
            .masters
            .get(master)
            .or_else(|| self.masters.values().next())
        else {
            return section;
        };
        if let Some(page) = self.page_layouts.get(layout) {
            section.page = *page;
        }
        for (story, slot) in [(header, &mut section.header), (footer, &mut section.footer)] {
            if let Some(story) = story {
                let mut blocks = Vec::new();
                for element in story.elements() {
                    self.block(element, &mut blocks, None);
                }
                *slot = blocks;
            }
        }
        section
    }

    fn block(&self, element: &Element, out: &mut Vec<Block>, list: Option<(Option<&str>, u8)>) {
        match element.local() {
            "p" | "h" => out.push(Block::Paragraph(self.paragraph(element, list))),
            "list" => {
                let style = element
                    .attr("text:style-name")
                    .or(list.and_then(|(style, _)| style));
                let level = list.map(|(_, level)| level + 1).unwrap_or(0);
                for item in element.elements() {
                    if matches!(item.local(), "list-item" | "list-header") {
                        for child in item.elements() {
                            self.block(child, out, Some((style, level)));
                        }
                    }
                }
            }
            "table" => out.push(Block::Table(self.table(element))),
            "section" | "soft-page-break" | "index-body" | "table-of-content"
            | "alphabetical-index" | "illustration-index" | "user-index" | "bibliography"
            | "object-index" | "table-index" => {
                for child in element.elements() {
                    self.block(child, out, list);
                }
            }
            _ => {}
        }
    }

    fn paragraph(&self, element: &Element, list: Option<(Option<&str>, u8)>) -> Paragraph {
        let (properties, text, outline, display) =
            self.paragraph_style(element.attr("text:style-name"));
        let lower = display.to_ascii_lowercase();
        let kind = if element.is("h") {
            let level = element
                .attr("text:outline-level")
                .and_then(|level| level.parse::<u8>().ok())
                .or(outline)
                .unwrap_or(1);
            ParagraphKind::Heading(level.clamp(1, 9))
        } else if lower == "title" {
            ParagraphKind::Title
        } else if lower.contains("preformatted") {
            ParagraphKind::Preformatted
        } else if lower.starts_with("quotation") {
            ParagraphKind::Quote
        } else {
            ParagraphKind::Normal
        };
        let mut paragraph = Paragraph {
            kind,
            alignment: properties.alignment,
            indent_left: properties.left.unwrap_or(0),
            indent_right: properties.right.unwrap_or(0),
            indent_first_line: properties.first.unwrap_or(0),
            space_before: properties.before,
            space_after: properties.after,
            line_spacing: properties.line,
            list: list.map(|(style, level)| ListItem {
                ordered: style
                    .and_then(|style| self.list_styles.get(style))
                    .and_then(|levels| levels.get(level as usize).copied())
                    .unwrap_or(false),
                level: level.min(8),
            }),
            page_break_before: properties.break_before.unwrap_or(false),
            keep_with_next: properties.keep_with_next.unwrap_or(false),
            shading: properties.background.clone(),
            inlines: Vec::new(),
        };
        if paragraph.list.is_some() {
            paragraph.indent_left = 0;
            paragraph.indent_first_line = 0;
        }
        self.inlines(element, &text, None, &mut paragraph);
        paragraph
    }

    fn inlines(
        &self,
        element: &Element,
        text: &TextProps,
        link: Option<&str>,
        out: &mut Paragraph,
    ) {
        for node in &element.children {
            match node {
                Node::Text(value) => {
                    // ODF collapses white space as XML does: runs become one space.
                    let mut collapsed = String::with_capacity(value.len());
                    let mut last_space = false;
                    for ch in value.chars() {
                        if ch.is_whitespace() {
                            if !last_space {
                                collapsed.push(' ');
                            }
                            last_space = true;
                        } else {
                            collapsed.push(ch);
                            last_space = false;
                        }
                    }
                    let mut format = text.format(&self.default_text);
                    format.link = link.map(str::to_string);
                    out.push_text(&collapsed, &format);
                }
                Node::Element(child) => match child.local() {
                    "span" => {
                        let mut properties = text.clone();
                        if let Some(name) = child.attr("text:style-name") {
                            for style in self.style_chain("text", name).iter().rev() {
                                properties = properties.merged(&style.text);
                            }
                        }
                        self.inlines(child, &properties, link, out);
                    }
                    "a" => {
                        let target = child.attr("xlink:href");
                        self.inlines(child, text, target.or(link), out);
                    }
                    "s" => {
                        let count = child
                            .attr("text:c")
                            .and_then(|c| c.parse::<usize>().ok())
                            .unwrap_or(1);
                        let mut format = text.format(&self.default_text);
                        format.link = link.map(str::to_string);
                        out.push_text(&" ".repeat(count.min(1000)), &format);
                    }
                    "tab" => out.inlines.push(Inline::Tab),
                    "line-break" => out.inlines.push(Inline::LineBreak),
                    "frame" => {
                        if let Some(image) = self.frame(child) {
                            out.inlines.push(Inline::Image(image));
                        }
                    }
                    "note" | "annotation" | "bookmark" | "bookmark-start" | "bookmark-end"
                    | "soft-page-break" | "reference-mark" | "change" | "change-start"
                    | "change-end" | "sequence-decls" => {}
                    _ => self.inlines(child, text, link, out),
                },
            }
        }
    }

    fn frame(&self, frame: &Element) -> Option<Image> {
        let image = frame.child("image")?;
        let data = if let Some(href) = image.attr("xlink:href") {
            self.pictures.get(href.trim_start_matches("./")).cloned()?
        } else {
            let binary = image.child("binary-data")?.text();
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(binary.split_whitespace().collect::<String>())
                .ok()?
        };
        let description = frame
            .child("desc")
            .or_else(|| frame.child("title"))
            .map(Element::text)
            .filter(|text| !text.is_empty());
        let mut image = Image::new(data, description)?;
        let emu = |name: &str| {
            frame
                .attr(name)
                .and_then(length_twips)
                .map(|v| v.max(0) as u64 * EMU_PER_TWIP)
        };
        if let (Some(width), Some(height)) = (emu("svg:width"), emu("svg:height")) {
            if width > 0 && height > 0 {
                image.width = width;
                image.height = height;
            }
        }
        Some(image)
    }

    fn table(&self, element: &Element) -> Table {
        let mut table = Table::default();
        let mut columns = Vec::new();
        let mut rows: Vec<(&Element, bool)> = Vec::new();
        fn collect<'a>(
            element: &'a Element,
            header: bool,
            columns: &mut Vec<&'a Element>,
            rows: &mut Vec<(&'a Element, bool)>,
        ) {
            for child in element.elements() {
                match child.local() {
                    "table-column" => columns.push(child),
                    "table-columns" | "table-column-group" => collect(child, header, columns, rows),
                    "table-header-rows" => collect(child, true, columns, rows),
                    "table-rows" | "table-row-group" => collect(child, header, columns, rows),
                    "table-row" => rows.push((child, header)),
                    _ => {}
                }
            }
        }
        collect(element, false, &mut columns, &mut rows);
        let mut widths = Vec::new();
        for column in columns {
            let repeat = column
                .attr("table:number-columns-repeated")
                .and_then(|r| r.parse::<usize>().ok())
                .unwrap_or(1)
                .min(64);
            let width = column
                .attr("table:style-name")
                .and_then(|name| {
                    self.styles
                        .get(&("table-column".to_string(), name.to_string()))
                })
                .and_then(|style| style.column_width)
                .unwrap_or(0)
                .max(0) as u32;
            widths.extend(std::iter::repeat_n(width, repeat));
        }
        // Columns a vertical merge still covers: column → (rows left, span).
        let mut pending_rows: HashMap<usize, (usize, u32)> = HashMap::new();
        for (row, header) in rows {
            let mut cells = Vec::new();
            let mut column = 0usize;
            let mut covered_across = 0u32;
            for cell in row.elements() {
                match cell.local() {
                    "table-cell" => {
                        let span = cell
                            .attr("table:number-columns-spanned")
                            .and_then(|s| s.parse::<u32>().ok())
                            .unwrap_or(1)
                            .max(1);
                        let rows_spanned = cell
                            .attr("table:number-rows-spanned")
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(1);
                        let mut blocks = Vec::new();
                        for child in cell.elements() {
                            self.block(child, &mut blocks, None);
                        }
                        let vertical_merge = if rows_spanned > 1 {
                            pending_rows.insert(column, (rows_spanned - 1, span));
                            VerticalMerge::Restart
                        } else {
                            VerticalMerge::None
                        };
                        let shading = cell
                            .attr("table:style-name")
                            .and_then(|name| {
                                self.styles
                                    .get(&("table-cell".to_string(), name.to_string()))
                            })
                            .and_then(|style| style.paragraph.background.clone());
                        cells.push(Cell {
                            blocks,
                            span,
                            vertical_merge,
                            shading,
                        });
                        column += span as usize;
                        covered_across = span - 1;
                    }
                    "covered-table-cell" => {
                        if covered_across > 0 {
                            covered_across -= 1;
                            continue;
                        }
                        match pending_rows.get_mut(&column) {
                            Some((remaining, span)) if *remaining > 0 => {
                                *remaining -= 1;
                                let span = *span;
                                cells.push(Cell {
                                    span,
                                    vertical_merge: VerticalMerge::Continue,
                                    ..Cell::default()
                                });
                                column += span as usize;
                                covered_across = span - 1;
                            }
                            _ => column += 1,
                        }
                    }
                    _ => {}
                }
            }
            table.rows.push(Row { cells, header });
        }
        let grid = table.grid_width();
        table.columns = if widths.len() == grid && widths.iter().all(|width| *width > 0) {
            widths
        } else {
            Table::even_columns(grid, PageSetup::default().text_width())
        };
        table.borders = true;
        table
    }
}

/// An ODF length (`2.5cm`, `0.5in`, `12pt`) in twips.
fn length_twips(value: &str) -> Option<i64> {
    let value = value.trim();
    let split = value
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == '+'))
        .unwrap_or(value.len());
    let number: f64 = value[..split].parse().ok()?;
    let twips = match &value[split..] {
        "in" => number * 1440.0,
        "cm" => number * 1440.0 / 2.54,
        "mm" => number * 144.0 / 2.54,
        "pt" => number * 20.0,
        "pc" => number * 240.0,
        "px" => number * 15.0,
        "" => number,
        _ => return None,
    };
    Some(twips.round() as i64)
}

fn points(value: &str) -> Option<f64> {
    length_twips(value)
        .map(|twips| twips as f64 / 20.0)
        .filter(|_| !value.ends_with('%'))
}

fn hex_color(value: &str) -> Option<String> {
    let hex = value.trim().strip_prefix('#')?;
    (hex.len() == 6 && hex.chars().all(|ch| ch.is_ascii_hexdigit()))
        .then(|| hex.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> FlowDocument {
        let mut heading = Paragraph::with_text("見出し");
        heading.kind = ParagraphKind::Heading(2);
        let mut body = Paragraph {
            alignment: Some(Alignment::Center),
            ..Paragraph::default()
        };
        body.push_text("plain  two spaces ", &CharFormat::default());
        body.push_text(
            "bold",
            &CharFormat {
                bold: true,
                color: Some("FF0000".into()),
                size: Some(28),
                ..CharFormat::default()
            },
        );
        body.inlines.push(Inline::Tab);
        body.push_text(
            "link",
            &CharFormat {
                link: Some("https://example.com/".into()),
                ..CharFormat::default()
            },
        );
        let mut one = Paragraph::with_text("one");
        one.list = Some(ListItem {
            ordered: true,
            level: 0,
        });
        let mut nested = Paragraph::with_text("nested");
        nested.list = Some(ListItem {
            ordered: true,
            level: 1,
        });
        let mut two = Paragraph::with_text("two");
        two.list = Some(ListItem {
            ordered: true,
            level: 0,
        });
        let png = {
            let mut data = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
            data.extend_from_slice(&2u32.to_be_bytes());
            data.extend_from_slice(&3u32.to_be_bytes());
            data.extend_from_slice(&[8, 2, 0, 0, 0, 0, 0, 0, 0]);
            data
        };
        let mut picture = Paragraph::default();
        picture
            .inlines
            .push(Inline::Image(Image::new(png, Some("alt".into())).unwrap()));
        let table = Table {
            columns: vec![2000, 2000, 2000],
            rows: vec![
                Row {
                    cells: vec![
                        Cell {
                            span: 2,
                            ..Cell::with_text("wide")
                        },
                        Cell {
                            vertical_merge: VerticalMerge::Restart,
                            ..Cell::with_text("tall")
                        },
                    ],
                    header: true,
                },
                Row {
                    cells: vec![
                        Cell::with_text("a"),
                        Cell::with_text("b"),
                        Cell {
                            vertical_merge: VerticalMerge::Continue,
                            ..Cell::default()
                        },
                    ],
                    header: false,
                },
            ],
            borders: true,
        };
        FlowDocument {
            title: Some("文書".into()),
            sections: vec![
                FlowSection {
                    blocks: vec![
                        Block::Paragraph(heading),
                        Block::Paragraph(body),
                        Block::Paragraph(one),
                        Block::Paragraph(nested),
                        Block::Paragraph(two),
                        Block::Paragraph(picture),
                        Block::Table(table),
                    ],
                    header: vec![Block::Paragraph(Paragraph::with_text("header"))],
                    ..FlowSection::default()
                },
                FlowSection {
                    blocks: vec![Block::Paragraph(Paragraph::with_text("landscape"))],
                    page: PageSetup {
                        width: 16838,
                        height: 11906,
                        ..PageSetup::default()
                    },
                    ..FlowSection::default()
                },
            ],
            ..FlowDocument::default()
        }
    }

    #[test]
    fn a_written_document_reads_back() {
        let bytes = write_odt(&sample()).unwrap();
        // The mimetype comes first, stored, as ODF requires.
        assert_eq!(&bytes[30..38], b"mimetype");
        assert_eq!(&bytes[38..38 + MIMETYPE.len()], MIMETYPE.as_bytes());
        let read = read_odt(&bytes).unwrap();
        assert_eq!(read.title.as_deref(), Some("文書"));
        assert_eq!(read.sections.len(), 2);
        assert!(read.sections[1].page.landscape());
        assert_eq!(read.sections[0].header.len(), 1);
        let paragraphs = read.paragraphs();
        assert_eq!(paragraphs[0].kind, ParagraphKind::Heading(2));
        assert_eq!(paragraphs[1].text(), "plain  two spaces bold\tlink");
        assert_eq!(paragraphs[1].alignment, Some(Alignment::Center));
        assert!(paragraphs[1].inlines.iter().any(|inline| matches!(
            inline,
            Inline::Text(text, format) if text == "bold" && format.bold && format.size == Some(28) && format.color.as_deref() == Some("FF0000")
        )));
        assert!(paragraphs[1].inlines.iter().any(|inline| matches!(
            inline,
            Inline::Text(text, format) if text == "link" && format.link.as_deref() == Some("https://example.com/")
        )));
        assert_eq!(
            paragraphs[2].list,
            Some(ListItem {
                ordered: true,
                level: 0
            })
        );
        assert_eq!(
            paragraphs[3].list,
            Some(ListItem {
                ordered: true,
                level: 1
            })
        );
        assert_eq!(
            paragraphs[4].list,
            Some(ListItem {
                ordered: true,
                level: 0
            })
        );
        let image = paragraphs[5]
            .inlines
            .iter()
            .find_map(|inline| match inline {
                Inline::Image(image) => Some(image.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(image.description.as_deref(), Some("alt"));
        let Block::Table(table) = &read.sections[0].blocks[6] else {
            panic!("a table")
        };
        assert!(table.rows[0].header);
        assert_eq!(table.rows[0].cells[0].span, 2);
        assert_eq!(
            table.rows[0].cells[1].vertical_merge,
            VerticalMerge::Restart
        );
        assert_eq!(
            table.rows[1].cells[2].vertical_merge,
            VerticalMerge::Continue
        );
    }
}
