//! A `.docx` package as a flow document: what every export starts from.
//!
//! Formatting is resolved as Word resolves it — document defaults, then the
//! paragraph style and the styles it is based on, then the character
//! style, then the run's own — so a format with no styles of its own (plain
//! RTF, HTML) still shows each run as it looked. Tracked deletions are
//! left out and insertions kept: what is exported is the document as it
//! reads with every change accepted, as Word saves to other formats.

use std::collections::HashMap;

use jade_opc::OpcPackage;

use crate::flow::*;
use crate::xml::{parse, Element, Node};

/// Reads `bytes`, a WordprocessingML package.
pub fn read_docx(bytes: &[u8]) -> Result<FlowDocument, String> {
    let package =
        OpcPackage::open(bytes).map_err(|error| format!("not a Word document: {error}"))?;
    let main = main_part(&package).unwrap_or_else(|| "word/document.xml".to_string());
    let xml = package
        .part_str(&main)
        .map_err(|error| error.to_string())?
        .ok_or("the document has no main part")?;
    let root = parse(&xml)?;
    let body = root.child("body").ok_or("the document has no body")?;
    let styles = package
        .part_str(&sibling(&main, "styles.xml"))
        .ok()
        .flatten()
        .and_then(|xml| parse(&xml).ok())
        .map(|root| Styles::read(&root))
        .unwrap_or_default();
    let numbering = package
        .part_str(&sibling(&main, "numbering.xml"))
        .ok()
        .flatten()
        .and_then(|xml| parse(&xml).ok())
        .map(|root| Numbering::read(&root))
        .unwrap_or_default();
    let reader = Reader {
        package: &package,
        styles,
        numbering,
    };
    let relationships = reader.relationships(&main);
    let mut document = FlowDocument {
        title: core_property(&package, "title"),
        author: core_property(&package, "creator"),
        default_font: reader.styles.defaults.0.font.clone(),
        default_size: reader.styles.defaults.0.size,
        language: reader.styles.language.clone(),
        sections: Vec::new(),
    };
    let mut section = FlowSection::default();
    for element in body.elements() {
        match element.local() {
            "p" => {
                let ends_section = element
                    .child("pPr")
                    .and_then(|properties| properties.child("sectPr"));
                reader.body_element(element, &relationships, &mut section.blocks);
                if let Some(properties) = ends_section {
                    reader.section_properties(properties, &main, &mut section);
                    inherit_stories(&document.sections, properties, &mut section);
                    document.sections.push(std::mem::take(&mut section));
                }
            }
            "sectPr" => {
                reader.section_properties(element, &main, &mut section);
                inherit_stories(&document.sections, element, &mut section);
            }
            _ => reader.body_element(element, &relationships, &mut section.blocks),
        }
    }
    document.sections.push(section);
    Ok(document)
}

/// A section that names no header (or footer) of its own has the one
/// before it, as Word links a new section's to the previous.
fn inherit_stories(sections: &[FlowSection], properties: &Element, section: &mut FlowSection) {
    let Some(previous) = sections.last() else {
        return;
    };
    let names = |kind: &str| {
        properties.children_named(kind).any(|reference| {
            reference
                .attr("w:type")
                .is_none_or(|kind| kind == "default")
        })
    };
    if !names("headerReference") {
        section.header = previous.header.clone();
    }
    if !names("footerReference") {
        section.footer = previous.footer.clone();
    }
}

fn main_part(package: &OpcPackage) -> Option<String> {
    let rels = package.part_str("_rels/.rels").ok()??;
    let root = parse(&rels).ok()?;
    let target = root
        .elements()
        .find(|relationship| {
            relationship
                .attr("Type")
                .is_some_and(|kind| kind.ends_with("/officeDocument"))
        })
        .and_then(|relationship| relationship.attr("Target"))
        .map(|target| target.trim_start_matches('/').to_string());
    target
}

fn sibling(part: &str, name: &str) -> String {
    match part.rsplit_once('/') {
        Some((directory, _)) => format!("{directory}/{name}"),
        None => name.to_string(),
    }
}

fn core_property(package: &OpcPackage, name: &str) -> Option<String> {
    let xml = package.part_str("docProps/core.xml").ok()??;
    let root = parse(&xml).ok()?;
    let value = root.child(name)?.text();
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// What a paragraph's properties resolve to, before its runs.
#[derive(Clone, Debug, Default)]
struct ParagraphProperties {
    alignment: Option<Alignment>,
    indent_left: Option<i32>,
    indent_right: Option<i32>,
    indent_first_line: Option<i32>,
    space_before: Option<u32>,
    space_after: Option<u32>,
    line_spacing: Option<LineSpacing>,
    outline_level: Option<u8>,
    keep_with_next: Option<bool>,
    page_break_before: Option<bool>,
    numbering: Option<(String, u8)>,
    shading: Option<String>,
}

impl ParagraphProperties {
    fn read(properties: &Element) -> Self {
        let mut out = Self::default();
        if let Some(value) = properties.child_val("jc") {
            out.alignment = match value {
                "center" => Some(Alignment::Center),
                "right" | "end" => Some(Alignment::Right),
                "both" | "distribute" | "lowKashida" | "mediumKashida" | "highKashida" => {
                    Some(Alignment::Justify)
                }
                _ => Some(Alignment::Left),
            };
        }
        if let Some(indent) = properties.child("ind") {
            let number = |name: &str| {
                indent
                    .attr(name)
                    .and_then(|value| value.parse::<i32>().ok())
            };
            out.indent_left = number("w:left").or_else(|| number("w:start"));
            out.indent_right = number("w:right").or_else(|| number("w:end"));
            if let Some(first) = number("w:firstLine") {
                out.indent_first_line = Some(first);
            }
            if let Some(hanging) = number("w:hanging") {
                out.indent_first_line = Some(-hanging);
            }
        }
        if let Some(spacing) = properties.child("spacing") {
            let number = |name: &str| {
                spacing
                    .attr(name)
                    .and_then(|value| value.parse::<u32>().ok())
            };
            out.space_before = number("w:before");
            out.space_after = number("w:after");
            if let Some(line) = number("w:line") {
                out.line_spacing = Some(match spacing.attr("w:lineRule") {
                    Some("exact") => LineSpacing::Exact(line),
                    Some("atLeast") => LineSpacing::AtLeast(line),
                    _ => LineSpacing::Multiple(line),
                });
            }
        }
        if let Some(level) = properties
            .child_val("outlineLvl")
            .and_then(|v| v.parse::<u8>().ok())
        {
            out.outline_level = Some(level);
        }
        out.keep_with_next = properties.flag("keepNext");
        out.page_break_before = properties.flag("pageBreakBefore");
        if let Some(numbering) = properties.child("numPr") {
            let id = numbering.child_val("numId").map(str::to_string);
            let level = numbering
                .child_val("ilvl")
                .and_then(|value| value.parse::<u8>().ok())
                .unwrap_or(0);
            if let Some(id) = id {
                out.numbering = Some((id, level));
            }
        }
        if let Some(fill) = properties
            .child("shd")
            .and_then(|shading| shading.attr("w:fill"))
        {
            if fill != "auto" && fill.len() == 6 {
                out.shading = Some(fill.to_ascii_uppercase());
            }
        }
        out
    }

    /// `self` with `over`'s settings on top.
    fn merged(&self, over: &Self) -> Self {
        Self {
            alignment: over.alignment.or(self.alignment),
            indent_left: over.indent_left.or(self.indent_left),
            indent_right: over.indent_right.or(self.indent_right),
            indent_first_line: over.indent_first_line.or(self.indent_first_line),
            space_before: over.space_before.or(self.space_before),
            space_after: over.space_after.or(self.space_after),
            line_spacing: over.line_spacing.or(self.line_spacing),
            outline_level: over.outline_level.or(self.outline_level),
            keep_with_next: over.keep_with_next.or(self.keep_with_next),
            page_break_before: over.page_break_before.or(self.page_break_before),
            numbering: over.numbering.clone().or_else(|| self.numbering.clone()),
            shading: over.shading.clone().or_else(|| self.shading.clone()),
        }
    }
}

/// A run's properties as each layer sets them: `None` leaves the layer
/// below's.
#[derive(Clone, Debug, Default)]
struct RunProperties {
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    vertical: Option<Option<bool>>,
    small_caps: Option<bool>,
    all_caps: Option<bool>,
    hidden: Option<bool>,
    size: Option<u32>,
    font: Option<String>,
    east_asian_font: Option<String>,
    color: Option<String>,
    highlight: Option<String>,
    shading: Option<String>,
}

impl RunProperties {
    fn read(properties: &Element) -> Self {
        let mut out = Self {
            bold: properties.flag("b"),
            italic: properties.flag("i"),
            strike: properties.flag("strike").or(properties.flag("dstrike")),
            small_caps: properties.flag("smallCaps"),
            all_caps: properties.flag("caps"),
            hidden: properties.flag("vanish"),
            ..Self::default()
        };
        if let Some(underline) = properties.child("u") {
            out.underline = Some(!matches!(underline.attr("w:val"), Some("none") | Some("0")));
        }
        if let Some(value) = properties.child_val("vertAlign") {
            out.vertical = Some(match value {
                "superscript" => Some(true),
                "subscript" => Some(false),
                _ => None,
            });
        }
        out.size = properties
            .child_val("sz")
            .and_then(|value| value.parse().ok());
        if let Some(fonts) = properties.child("rFonts") {
            out.font = fonts
                .attr("w:ascii")
                .or_else(|| fonts.attr("w:hAnsi"))
                .map(str::to_string);
            out.east_asian_font = fonts.attr("w:eastAsia").map(str::to_string);
        }
        if let Some(color) = properties.child_val("color") {
            if color != "auto" && color.len() == 6 {
                out.color = Some(color.to_ascii_uppercase());
            }
        }
        if let Some(highlight) = properties.child_val("highlight") {
            if highlight != "none" {
                out.highlight = Some(highlight.to_string());
            }
        }
        if let Some(fill) = properties
            .child("shd")
            .and_then(|shading| shading.attr("w:fill"))
        {
            if fill != "auto" && fill.len() == 6 {
                out.shading = Some(fill.to_ascii_uppercase());
            }
        }
        out
    }

    fn merged(&self, over: &Self) -> Self {
        Self {
            bold: over.bold.or(self.bold),
            italic: over.italic.or(self.italic),
            underline: over.underline.or(self.underline),
            strike: over.strike.or(self.strike),
            vertical: over.vertical.or(self.vertical),
            small_caps: over.small_caps.or(self.small_caps),
            all_caps: over.all_caps.or(self.all_caps),
            hidden: over.hidden.or(self.hidden),
            size: over.size.or(self.size),
            font: over.font.clone().or_else(|| self.font.clone()),
            east_asian_font: over
                .east_asian_font
                .clone()
                .or_else(|| self.east_asian_font.clone()),
            color: over.color.clone().or_else(|| self.color.clone()),
            highlight: over.highlight.clone().or_else(|| self.highlight.clone()),
            shading: over.shading.clone().or_else(|| self.shading.clone()),
        }
    }

    /// The flow format, with what the document's defaults already say left
    /// out: a run in the body face at the body size names neither.
    fn format(&self, defaults: &RunProperties) -> CharFormat {
        let differs = |value: &Option<String>, default: &Option<String>| {
            value
                .clone()
                .filter(|value| Some(value) != default.as_ref())
        };
        CharFormat {
            bold: self.bold.unwrap_or(false),
            italic: self.italic.unwrap_or(false),
            underline: self.underline.unwrap_or(false),
            strike: self.strike.unwrap_or(false),
            superscript: self.vertical.flatten() == Some(true),
            subscript: self.vertical.flatten() == Some(false),
            small_caps: self.small_caps.unwrap_or(false),
            all_caps: self.all_caps.unwrap_or(false),
            hidden: self.hidden.unwrap_or(false),
            size: self.size.filter(|size| Some(*size) != defaults.size),
            font: differs(&self.font, &defaults.font),
            east_asian_font: differs(&self.east_asian_font, &defaults.east_asian_font),
            color: self.color.clone(),
            highlight: self.highlight.clone(),
            shading: self.shading.clone(),
            link: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Style {
    name: String,
    based_on: Option<String>,
    paragraph: ParagraphProperties,
    run: RunProperties,
}

#[derive(Default)]
struct Styles {
    defaults: (RunProperties, ParagraphProperties),
    styles: HashMap<String, Style>,
    default_paragraph: Option<String>,
    language: Option<String>,
}

impl Styles {
    fn read(root: &Element) -> Self {
        let mut out = Self::default();
        if let Some(defaults) = root.child("docDefaults") {
            if let Some(run) = defaults.child("rPrDefault").and_then(|d| d.child("rPr")) {
                out.defaults.0 = RunProperties::read(run);
                out.language = run
                    .child("lang")
                    .and_then(|lang| lang.attr("w:val"))
                    .map(str::to_string);
            }
            if let Some(paragraph) = defaults.child("pPrDefault").and_then(|d| d.child("pPr")) {
                out.defaults.1 = ParagraphProperties::read(paragraph);
            }
        }
        for style in root.children_named("style") {
            let Some(id) = style.attr("w:styleId") else {
                continue;
            };
            if style.attr("w:type") == Some("paragraph")
                && style
                    .attr("w:default")
                    .is_some_and(|v| v == "1" || v == "true")
            {
                out.default_paragraph = Some(id.to_string());
            }
            out.styles.insert(
                id.to_string(),
                Style {
                    name: style.child_val("name").unwrap_or(id).to_string(),
                    based_on: style.child_val("basedOn").map(str::to_string),
                    paragraph: style
                        .child("pPr")
                        .map(ParagraphProperties::read)
                        .unwrap_or_default(),
                    run: style
                        .child("rPr")
                        .map(RunProperties::read)
                        .unwrap_or_default(),
                },
            );
        }
        out
    }

    /// The chain from `id` up through what it is based on, the root last.
    fn chain(&self, id: &str) -> Vec<&Style> {
        let mut out = Vec::new();
        let mut next = Some(id.to_string());
        while let Some(id) = next {
            let Some(style) = self.styles.get(&id) else {
                break;
            };
            if out.len() > 16 {
                break;
            }
            out.push(style);
            next = style.based_on.clone();
        }
        out
    }

    fn paragraph_style(&self, id: Option<&str>) -> (ParagraphProperties, RunProperties, String) {
        let id = id
            .map(str::to_string)
            .or_else(|| self.default_paragraph.clone());
        let mut paragraph = self.defaults.1.clone();
        let mut run = self.defaults.0.clone();
        let mut name = String::new();
        if let Some(id) = id {
            let chain = self.chain(&id);
            if let Some(first) = chain.first() {
                name = first.name.clone();
            }
            for style in chain.iter().rev() {
                paragraph = paragraph.merged(&style.paragraph);
                run = run.merged(&style.run);
            }
        }
        (paragraph, run, name)
    }

    fn character_style(&self, id: &str) -> RunProperties {
        let mut run = RunProperties::default();
        for style in self.chain(id).iter().rev() {
            run = run.merged(&style.run);
        }
        run
    }
}

#[derive(Default)]
struct Numbering {
    /// numId → abstractNumId.
    instances: HashMap<String, String>,
    /// abstractNumId → each level's numFmt.
    formats: HashMap<String, Vec<String>>,
}

impl Numbering {
    fn read(root: &Element) -> Self {
        let mut out = Self::default();
        for abstract_num in root.children_named("abstractNum") {
            let Some(id) = abstract_num.attr("w:abstractNumId") else {
                continue;
            };
            let mut formats = vec![String::from("decimal"); 9];
            for level in abstract_num.children_named("lvl") {
                let index = level
                    .attr("w:ilvl")
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                if let (Some(slot), Some(format)) =
                    (formats.get_mut(index), level.child_val("numFmt"))
                {
                    *slot = format.to_string();
                }
            }
            out.formats.insert(id.to_string(), formats);
        }
        for num in root.children_named("num") {
            if let (Some(id), Some(abstract_id)) =
                (num.attr("w:numId"), num.child_val("abstractNumId"))
            {
                out.instances
                    .insert(id.to_string(), abstract_id.to_string());
            }
        }
        out
    }

    fn list_item(&self, id: &str, level: u8) -> Option<ListItem> {
        if id == "0" {
            return None;
        }
        let format = self
            .instances
            .get(id)
            .and_then(|abstract_id| self.formats.get(abstract_id))
            .and_then(|formats| formats.get(level as usize))
            .map(String::as_str)
            .unwrap_or("decimal");
        if format == "none" {
            return None;
        }
        Some(ListItem {
            ordered: format != "bullet",
            level,
        })
    }
}

struct Reader<'a> {
    package: &'a OpcPackage,
    styles: Styles,
    numbering: Numbering,
}

type Relationships = HashMap<String, (String, String)>;

impl Reader<'_> {
    /// `part`'s relationships: id → (target part or URL, mode).
    fn relationships(&self, part: &str) -> Relationships {
        let name = jade_opc::rels_part_name(part);
        let mut out = HashMap::new();
        let Some(xml) = self.package.part_str(&name).ok().flatten() else {
            return out;
        };
        let Ok(root) = parse(&xml) else {
            return out;
        };
        for relationship in root.elements() {
            let (Some(id), Some(target)) = (relationship.attr("Id"), relationship.attr("Target"))
            else {
                continue;
            };
            let external = relationship.attr("TargetMode") == Some("External");
            let target = if external {
                target.to_string()
            } else {
                jade_opc::resolve_target(part, target)
            };
            out.insert(
                id.to_string(),
                (
                    target,
                    if external {
                        "External".into()
                    } else {
                        String::new()
                    },
                ),
            );
        }
        out
    }

    fn section_properties(&self, properties: &Element, main: &str, section: &mut FlowSection) {
        let number = |element: Option<&Element>, name: &str| {
            element
                .and_then(|element| element.attr(name))
                .and_then(|value| value.parse::<i64>().ok())
                .map(|value| value.max(0) as u32)
        };
        let size = properties.child("pgSz");
        let margins = properties.child("pgMar");
        let defaults = PageSetup::default();
        section.page = PageSetup {
            width: number(size, "w:w").unwrap_or(defaults.width),
            height: number(size, "w:h").unwrap_or(defaults.height),
            top: number(margins, "w:top").unwrap_or(defaults.top),
            right: number(margins, "w:right").unwrap_or(defaults.right),
            bottom: number(margins, "w:bottom").unwrap_or(defaults.bottom),
            left: number(margins, "w:left").unwrap_or(defaults.left),
            header: number(margins, "w:header").unwrap_or(defaults.header),
            footer: number(margins, "w:footer").unwrap_or(defaults.footer),
        };
        let relationships = self.relationships(main);
        for (kind, slot) in [
            ("headerReference", &mut section.header),
            ("footerReference", &mut section.footer),
        ] {
            let reference = properties.children_named(kind).find(|reference| {
                reference
                    .attr("w:type")
                    .is_none_or(|kind| kind == "default")
            });
            let Some(target) = reference
                .and_then(|reference| reference.attr("r:id"))
                .and_then(|id| relationships.get(id))
                .map(|(target, _)| target.clone())
            else {
                continue;
            };
            let Some(root) = self
                .package
                .part_str(&target)
                .ok()
                .flatten()
                .and_then(|xml| parse(&xml).ok())
            else {
                continue;
            };
            let story_relationships = self.relationships(&target);
            let mut blocks = Vec::new();
            for element in root.elements() {
                self.body_element(element, &story_relationships, &mut blocks);
            }
            *slot = blocks;
        }
    }

    fn body_element(&self, element: &Element, relationships: &Relationships, out: &mut Vec<Block>) {
        match element.local() {
            "p" => out.push(Block::Paragraph(self.paragraph(element, relationships))),
            "tbl" => out.push(Block::Table(self.table(element, relationships))),
            // Content controls, tracked insertions and custom XML wrap blocks.
            "sdt" => {
                if let Some(content) = element.child("sdtContent") {
                    for child in content.elements() {
                        self.body_element(child, relationships, out);
                    }
                }
            }
            "ins" | "customXml" | "smartTag" => {
                for child in element.elements() {
                    self.body_element(child, relationships, out);
                }
            }
            _ => {}
        }
    }

    fn paragraph(&self, element: &Element, relationships: &Relationships) -> Paragraph {
        let direct = element.child("pPr");
        let style_id = direct.and_then(|properties| properties.child_val("pStyle"));
        let (style_paragraph, style_run, style_name) = self.styles.paragraph_style(style_id);
        let own = direct.map(ParagraphProperties::read).unwrap_or_default();
        let resolved = style_paragraph.merged(&own);
        let name = style_name.to_ascii_lowercase();
        let kind = if name == "title" {
            ParagraphKind::Title
        } else if let Some(level) = name
            .strip_prefix("heading ")
            .and_then(|level| level.parse::<u8>().ok())
        {
            ParagraphKind::Heading(level.clamp(1, 9))
        } else if let Some(level) = resolved.outline_level.filter(|level| *level < 9) {
            ParagraphKind::Heading(level + 1)
        } else if name.contains("preformatted") || name == "plain text" {
            ParagraphKind::Preformatted
        } else if name == "quote" || name == "block text" {
            ParagraphKind::Quote
        } else {
            ParagraphKind::Normal
        };
        let list = resolved
            .numbering
            .as_ref()
            .and_then(|(id, level)| self.numbering.list_item(id, *level));
        let mut paragraph = Paragraph {
            kind,
            alignment: resolved.alignment,
            indent_left: resolved.indent_left.unwrap_or(0),
            indent_right: resolved.indent_right.unwrap_or(0),
            indent_first_line: resolved.indent_first_line.unwrap_or(0),
            space_before: resolved.space_before,
            space_after: resolved.space_after,
            line_spacing: resolved.line_spacing,
            list,
            page_break_before: resolved.page_break_before.unwrap_or(false),
            keep_with_next: resolved.keep_with_next.unwrap_or(false),
            shading: resolved.shading.clone(),
            inlines: Vec::new(),
        };
        // The paragraph mark's own run properties apply to no text.
        let mut state = RunState::default();
        self.inlines(
            element,
            relationships,
            &style_run,
            None,
            &mut paragraph,
            &mut state,
        );
        paragraph
    }

    fn inlines(
        &self,
        element: &Element,
        relationships: &Relationships,
        paragraph_run: &RunProperties,
        link: Option<&str>,
        out: &mut Paragraph,
        state: &mut RunState,
    ) {
        for child in element.elements() {
            match child.local() {
                "r" => self.run(child, relationships, paragraph_run, link, out, state),
                "hyperlink" => {
                    let target = child
                        .attr("r:id")
                        .and_then(|id| relationships.get(id))
                        .map(|(target, _)| target.clone())
                        .or_else(|| child.attr("w:anchor").map(|anchor| format!("#{anchor}")));
                    self.inlines(
                        child,
                        relationships,
                        paragraph_run,
                        target.as_deref().or(link),
                        out,
                        state,
                    );
                }
                "fldSimple" => {
                    let instruction = child.attr("w:instr").unwrap_or("");
                    let target = hyperlink_instruction(instruction);
                    self.inlines(
                        child,
                        relationships,
                        paragraph_run,
                        target.as_deref().or(link),
                        out,
                        state,
                    );
                }
                // Tracked deletions read as accepted: gone.
                "del" | "moveFrom" => {}
                "ins" | "moveTo" | "smartTag" | "customXml" | "bdo" | "dir" => {
                    self.inlines(child, relationships, paragraph_run, link, out, state)
                }
                "sdt" => {
                    if let Some(content) = child.child("sdtContent") {
                        self.inlines(content, relationships, paragraph_run, link, out, state);
                    }
                }
                _ => {}
            }
        }
    }

    fn run(
        &self,
        run: &Element,
        relationships: &Relationships,
        paragraph_run: &RunProperties,
        link: Option<&str>,
        out: &mut Paragraph,
        state: &mut RunState,
    ) {
        let direct = run.child("rPr");
        let mut properties = paragraph_run.clone();
        if let Some(style) = direct.and_then(|properties| properties.child_val("rStyle")) {
            properties = properties.merged(&self.styles.character_style(style));
        }
        if let Some(direct) = direct {
            properties = properties.merged(&RunProperties::read(direct));
        }
        let mut format = properties.format(&self.styles.defaults.0);
        format.link = state
            .field_link
            .clone()
            .or_else(|| link.map(str::to_string));
        for child in run.elements() {
            match child.local() {
                "fldChar" => match child.attr("w:fldCharType") {
                    Some("begin") => {
                        state.depth += 1;
                        state.in_instruction = true;
                        state.instruction.clear();
                    }
                    Some("separate") => {
                        state.in_instruction = false;
                        if state.depth == 1 {
                            state.field_link = hyperlink_instruction(&state.instruction);
                            format.link = state
                                .field_link
                                .clone()
                                .or_else(|| link.map(str::to_string));
                        }
                    }
                    Some("end") => {
                        state.depth = state.depth.saturating_sub(1);
                        state.in_instruction = false;
                        if state.depth == 0 {
                            state.field_link = None;
                            format.link = link.map(str::to_string);
                        }
                    }
                    _ => {}
                },
                "instrText" => {
                    if state.in_instruction {
                        state.instruction.push_str(&child.text());
                    }
                }
                _ if state.in_instruction => {}
                "t" => out.push_text(&child.text(), &format),
                "tab" => out.inlines.push(Inline::Tab),
                "br" => match child.attr("w:type") {
                    Some("page") => out.inlines.push(Inline::PageBreak),
                    Some("column") => out.inlines.push(Inline::LineBreak),
                    _ => out.inlines.push(Inline::LineBreak),
                },
                "cr" => out.inlines.push(Inline::LineBreak),
                "noBreakHyphen" => out.push_text("\u{2011}", &format),
                "softHyphen" => {}
                "sym" => {
                    if let Some(ch) = child
                        .attr("w:char")
                        .and_then(|code| u32::from_str_radix(code, 16).ok())
                        .and_then(|code| {
                            char::from_u32(if code >= 0xf000 { code - 0xf000 } else { code })
                        })
                    {
                        out.push_text(&ch.to_string(), &format);
                    }
                }
                "drawing" | "pict" | "object" => {
                    if let Some(image) = self.picture(child, relationships) {
                        out.inlines.push(Inline::Image(image));
                    }
                }
                "AlternateContent" => {
                    let chosen = child.child("Choice").or_else(|| child.child("Fallback"));
                    if let Some(chosen) = chosen {
                        for inner in chosen.elements() {
                            if matches!(inner.local(), "drawing" | "pict") {
                                if let Some(image) = self.picture(inner, relationships) {
                                    out.inlines.push(Inline::Image(image));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn picture(&self, element: &Element, relationships: &Relationships) -> Option<Image> {
        let (id, description) = if let Some(blip) = element.find("blip") {
            let description = element
                .find("docPr")
                .and_then(|properties| {
                    properties
                        .attr("descr")
                        .or_else(|| properties.attr("title"))
                })
                .filter(|text| !text.is_empty())
                .map(str::to_string);
            (blip.attr("r:embed")?.to_string(), description)
        } else {
            let data = element.find("imagedata")?;
            (
                data.attr("r:id")?.to_string(),
                data.attr("o:title").map(str::to_string),
            )
        };
        let (target, mode) = relationships.get(&id)?;
        if mode == "External" {
            return None;
        }
        let data = self.package.part(target)?.to_vec();
        let mut image = Image::new(data, description)?;
        if let Some(extent) = element.find("extent") {
            let size = |name: &str| {
                extent
                    .attr(name)
                    .and_then(|value| value.parse::<u64>().ok())
            };
            if let (Some(width), Some(height)) = (size("cx"), size("cy")) {
                if width > 0 && height > 0 {
                    image.width = width;
                    image.height = height;
                }
            }
        } else if let Some(shape) = element.find("shape") {
            if let Some((width, height)) = shape.attr("style").and_then(vml_size) {
                image.width = width;
                image.height = height;
            }
        }
        Some(image)
    }

    fn table(&self, element: &Element, relationships: &Relationships) -> Table {
        let columns: Vec<u32> = element
            .child("tblGrid")
            .map(|grid| {
                grid.children_named("gridCol")
                    .map(|column| column.attr("w:w").and_then(|w| w.parse().ok()).unwrap_or(0))
                    .collect()
            })
            .unwrap_or_default();
        let properties = element.child("tblPr");
        let style_borders = properties
            .and_then(|properties| properties.child_val("tblStyle"))
            .map(|style| {
                let lower = style.to_ascii_lowercase();
                lower.contains("grid") || lower.starts_with("table") && lower != "tablenormal"
            })
            .unwrap_or(false);
        let direct_borders = properties
            .and_then(|properties| properties.child("tblBorders"))
            .map(|borders| {
                borders
                    .elements()
                    .any(|edge| !matches!(edge.attr("w:val"), Some("none") | Some("nil") | None))
            });
        let mut table = Table {
            columns,
            rows: Vec::new(),
            borders: direct_borders.unwrap_or(style_borders),
        };
        for row in element.children_named("tr") {
            let header = row
                .child("trPr")
                .and_then(|properties| properties.flag("tblHeader"))
                .unwrap_or(false);
            let mut cells = Vec::new();
            for cell in row.elements() {
                let cell = match cell.local() {
                    "tc" => cell,
                    "sdt" => match cell
                        .child("sdtContent")
                        .and_then(|content| content.child("tc"))
                    {
                        Some(cell) => cell,
                        None => continue,
                    },
                    _ => continue,
                };
                let properties = cell.child("tcPr");
                let span = properties
                    .and_then(|properties| properties.child_val("gridSpan"))
                    .and_then(|span| span.parse().ok())
                    .unwrap_or(1);
                let vertical_merge = match properties
                    .and_then(|properties| properties.child("vMerge"))
                {
                    Some(merge) if merge.attr("w:val") == Some("restart") => VerticalMerge::Restart,
                    Some(_) => VerticalMerge::Continue,
                    None => VerticalMerge::None,
                };
                let shading = properties
                    .and_then(|properties| properties.child("shd"))
                    .and_then(|shading| shading.attr("w:fill"))
                    .filter(|fill| *fill != "auto" && fill.len() == 6)
                    .map(|fill| fill.to_ascii_uppercase());
                let mut blocks = Vec::new();
                for child in cell.elements() {
                    self.body_element(child, relationships, &mut blocks);
                }
                cells.push(Cell {
                    blocks,
                    span,
                    vertical_merge,
                    shading,
                });
            }
            table.rows.push(Row { cells, header });
        }
        table
    }
}

#[derive(Default)]
struct RunState {
    depth: usize,
    in_instruction: bool,
    instruction: String,
    field_link: Option<String>,
}

/// The target of a `HYPERLINK` field instruction.
pub(crate) fn hyperlink_instruction(instruction: &str) -> Option<String> {
    let rest = instruction.trim().strip_prefix("HYPERLINK")?.trim();
    let mut anchor = false;
    let mut words = Vec::new();
    let mut chars = rest.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            let mut word = String::new();
            for ch in chars.by_ref() {
                if ch == '"' {
                    break;
                }
                word.push(ch);
            }
            words.push(word);
        } else if !ch.is_whitespace() {
            let mut word = ch.to_string();
            while let Some(&next) = chars.peek() {
                if next.is_whitespace() {
                    break;
                }
                word.push(next);
                chars.next();
            }
            words.push(word);
        }
    }
    let mut url = None;
    let mut iter = words.into_iter();
    while let Some(word) = iter.next() {
        if word == "\\l" {
            anchor = true;
            if let Some(target) = iter.next() {
                return Some(format!("#{target}"));
            }
        } else if word.starts_with('\\') {
            iter.next();
        } else if url.is_none() {
            url = Some(word);
        }
    }
    let _ = anchor;
    url
}

/// A VML shape's `width:…;height:…` style in EMU.
fn vml_size(style: &str) -> Option<(u64, u64)> {
    let mut width = None;
    let mut height = None;
    for declaration in style.split(';') {
        let Some((name, value)) = declaration.split_once(':') else {
            continue;
        };
        let emu = css_length_emu(value.trim());
        match name.trim() {
            "width" => width = emu,
            "height" => height = emu,
            _ => {}
        }
    }
    Some((width?, height?))
}

/// A CSS length in EMU.
pub(crate) fn css_length_emu(value: &str) -> Option<u64> {
    let value = value.trim();
    let split = value
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.' || ch == '-'))
        .unwrap_or(value.len());
    let number: f64 = value[..split].parse().ok()?;
    let emu = match &value[split..] {
        "pt" => number * 12700.0,
        "in" => number * 914400.0,
        "cm" => number * 360000.0,
        "mm" => number * 36000.0,
        "pc" => number * 152400.0,
        "" | "px" => number * 9525.0,
        _ => return None,
    };
    (emu > 0.0).then_some(emu as u64)
}

/// The text of the elements under `node`, for callers that want words.
#[allow(dead_code)]
pub(crate) fn node_text(node: &Node) -> String {
    match node {
        Node::Text(text) => text.clone(),
        Node::Element(element) => element.text(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docx_write::write_docx;

    #[test]
    fn a_written_document_reads_back() {
        let mut heading = Paragraph::with_text("見出し");
        heading.kind = ParagraphKind::Heading(1);
        let mut body = Paragraph::default();
        body.push_text("plain ", &CharFormat::default());
        body.push_text(
            "bold",
            &CharFormat {
                bold: true,
                color: Some("FF0000".into()),
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
        let mut item = Paragraph::with_text("one");
        item.list = Some(ListItem {
            ordered: true,
            level: 0,
        });
        let table = Table {
            columns: vec![2000, 2000],
            rows: vec![Row {
                cells: vec![Cell::with_text("A1"), Cell::with_text("B1")],
                header: true,
            }],
            borders: true,
        };
        let mut section = FlowSection {
            blocks: vec![
                Block::Paragraph(heading),
                Block::Paragraph(body),
                Block::Paragraph(item),
                Block::Table(table),
            ],
            header: vec![Block::Paragraph(Paragraph::with_text("Header text"))],
            ..FlowSection::default()
        };
        section.page.width = 16838;
        section.page.height = 11906;
        let document = FlowDocument {
            title: Some("Report".into()),
            sections: vec![section],
            ..FlowDocument::default()
        };
        let bytes = write_docx(&document).unwrap();
        let read = read_docx(&bytes).unwrap();
        assert_eq!(read.title.as_deref(), Some("Report"));
        let section = &read.sections[0];
        assert!(section.page.landscape());
        assert_eq!(section.header.len(), 1);
        let Block::Paragraph(heading) = &section.blocks[0] else {
            panic!("a heading")
        };
        assert_eq!(heading.kind, ParagraphKind::Heading(1));
        let Block::Paragraph(body) = &section.blocks[1] else {
            panic!("a paragraph")
        };
        assert_eq!(body.text(), "plain bold\tlink");
        let bold = body
            .inlines
            .iter()
            .find_map(|inline| match inline {
                Inline::Text(text, format) if text == "bold" => Some(format),
                _ => None,
            })
            .unwrap();
        assert!(bold.bold);
        assert_eq!(bold.color.as_deref(), Some("FF0000"));
        let link = body
            .inlines
            .iter()
            .find_map(|inline| match inline {
                Inline::Text(text, format) if text == "link" => format.link.clone(),
                _ => None,
            })
            .unwrap();
        assert_eq!(link, "https://example.com/");
        let Block::Paragraph(item) = &section.blocks[2] else {
            panic!("a list item")
        };
        assert_eq!(item.list.map(|item| item.ordered), Some(true));
        let Block::Table(table) = &section.blocks[3] else {
            panic!("a table")
        };
        assert!(table.borders);
        assert!(table.rows[0].header);
        assert_eq!(table.rows[0].cells.len(), 2);
    }

    #[test]
    fn hyperlink_fields_name_their_targets() {
        assert_eq!(
            hyperlink_instruction(r#" HYPERLINK "https://example.com/" \o "tip" "#).as_deref(),
            Some("https://example.com/")
        );
        assert_eq!(
            hyperlink_instruction(r#"HYPERLINK \l "_Toc1""#).as_deref(),
            Some("#_Toc1")
        );
    }
}
