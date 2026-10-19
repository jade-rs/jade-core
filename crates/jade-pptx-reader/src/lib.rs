//! Reads the shape of a `.pptx`: how large a slide is, and which parts the
//! slides are, in presentation order.
//!
//! This is the first slice of pptx support and deliberately stops before the
//! slide body. Everything downstream — the shape tree, inheritance from layout
//! and master, and the geometry that turns into draw commands — is designed in
//! `docs/pptx-plan.md`; this crate exists so that plan has a place to land and
//! so the package-level facts are already tested when it does.

pub mod chart;
pub mod error;
mod shape_guides;
pub mod slide;
mod slide_theme;
pub mod table_style;
pub mod theme;
pub use slide_theme::{SlideTheme, ThemeRef};
mod xml_tree;

use jade_opc::{local_name, OpcPackage};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

pub use chart::{parse_chart_xml, ChartContent, ChartSeries, ChartStyle};
pub use error::{PptxError, Result};
pub use shape_guides::ShapeGuides;
pub use slide::{
    diagram_drawing_rel_id, parse_diagram_drawing_xml, Chart, CustomGeometry, GeometryPath,
    GradientPath, LineEnd, PathCommand,
};
pub use slide::{parse_default_text_style, parse_slide_xml, TextVertical};
pub use slide::{
    Autofit, Bullet, CellBorders, Color, Fill, GradientStop, GroupShape, GroupTransform, Hyperlink,
    LevelStyle, ListStyle, Outline, Picture, Placeholder, RunProps, Shadow, Shape, ShapeItem,
    ShapeStyle, SlideContent, SourceRect, Spacing, Table, TableCell, TableRow, TextAlign,
    TextAnchor, TextBody, TextBodyPropertyPresence, TextInsets, TextParagraph, TextRun, TextStyles,
    Transform,
};
pub use table_style::{parse_table_styles, TablePartStyle, TableStyle, TableStyleFlags};
pub use theme::{
    parse_color_map, parse_theme_colors, parse_theme_effect_styles, parse_theme_fill_styles,
    parse_theme_fonts, parse_theme_line_styles, ThemeColors, ThemeFonts,
};

/// English Metric Units per inch — the unit every pptx coordinate is in.
pub const EMU_PER_INCH: f64 = 914_400.0;

/// EMU to CSS pixels at 96 dpi.
pub fn emu_to_px(emu: i64) -> f64 {
    emu as f64 / EMU_PER_INCH * 96.0
}

const OFFICE_DOCUMENT: &str = "/officeDocument";
const SLIDE: &str = "/slide";

/// Slide dimensions in EMU, as `<p:sldSz>` states them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlideSize {
    pub width_emu: i64,
    pub height_emu: i64,
}

impl Default for SlideSize {
    /// PowerPoint's 16:9 default, 13.333 × 7.5 inches.
    fn default() -> Self {
        Self {
            width_emu: 12_192_000,
            height_emu: 6_858_000,
        }
    }
}

impl SlideSize {
    pub fn width_px(&self) -> f64 {
        emu_to_px(self.width_emu)
    }

    pub fn height_px(&self) -> f64 {
        emu_to_px(self.height_emu)
    }
}

/// One slide, identified but not yet parsed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slide {
    /// The `<p:sldId>` id, which is what the file uses to refer to the slide.
    pub id: Option<u32>,
    /// The part name inside the archive, e.g. `ppt/slides/slide1.xml`.
    pub part_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Presentation {
    pub slide_size: SlideSize,
    /// Slides in presentation order.
    pub slides: Vec<Slide>,
}

/// Reads a presentation's package-level structure from `.pptx` bytes.
pub fn read_pptx(bytes: &[u8]) -> Result<Presentation> {
    let package = OpcPackage::open(bytes)?;

    let presentation_part = match package.part_str("_rels/.rels")? {
        Some(xml) => jade_opc::parse_relationships(&xml)?
            .target_by_type_suffix(OFFICE_DOCUMENT)
            .map(|target| jade_opc::resolve_target("", target))
            .unwrap_or_else(|| "ppt/presentation.xml".to_string()),
        None => "ppt/presentation.xml".to_string(),
    };

    let presentation_xml = package.required_part_str(&presentation_part)?;
    let relationships = match package.part_str(&jade_opc::rels_part_name(&presentation_part))? {
        Some(xml) => jade_opc::parse_relationships(&xml)?,
        None => Default::default(),
    };

    let (slide_size, slide_ids) = parse_presentation(&presentation_xml)?;

    let mut slides = Vec::new();
    for entry in slide_ids {
        let Some(target) = entry
            .relationship_id
            .as_deref()
            .and_then(|id| relationships.target(id))
        else {
            continue;
        };
        let part_name = jade_opc::resolve_target(&presentation_part, target);
        if package.part(&part_name).is_none() {
            continue;
        }
        slides.push(Slide {
            id: entry.id,
            part_name,
        });
    }

    // A presentation whose `sldIdLst` is unreadable still has slide parts. One
    // slide beats reporting an empty deck, and `/slide` is the relationship
    // type that finds them.
    if slides.is_empty() {
        if let Some(target) = relationships.target_by_type_suffix(SLIDE) {
            let part_name = jade_opc::resolve_target(&presentation_part, target);
            if package.part(&part_name).is_some() {
                slides.push(Slide {
                    id: None,
                    part_name,
                });
            }
        }
    }

    Ok(Presentation { slide_size, slides })
}

/// A deck read all the way down to each slide's shape tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deck {
    pub slide_size: SlideSize,
    pub slides: Vec<DeckSlide>,
    pub theme_colors: ThemeColors,
    #[serde(default)]
    pub theme_fonts: ThemeFonts,
    /// `<p:defaultTextStyle>`: the last stop of text inheritance for a shape
    /// that is not a placeholder.
    #[serde(default)]
    pub default_text_style: ListStyle,
    /// The deck's own table styles from `ppt/tableStyles.xml`. Built-in
    /// styles are referenced by GUID and never written into the package.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub table_styles: Vec<TableStyle>,
    /// The theme's `fillStyleLst`, what a shape style's `fillRef idx` 1–3
    /// names, with `phClr` standing for the reference colour.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub theme_fill_styles: Vec<Fill>,
    /// The theme's `bgFillStyleLst` (`fillRef idx` 1001–1003).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub theme_background_fill_styles: Vec<Fill>,
    /// Theme lnStyleLst; lnRef idx is one-based and phClr uses its reference color.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub theme_line_styles: Vec<Outline>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeckSlide {
    pub id: Option<u32>,
    pub part_name: String,
    pub content: SlideContent,
    /// The slide's `slideLayout` shape tree — where placeholder geometry a
    /// slide leaves implicit actually lives. `None` when the deck has no
    /// readable layout part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_content: Option<SlideContent>,
    /// The layout's `slideMaster` shape tree, the last stop of placeholder
    /// inheritance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_content: Option<SlideContent>,
    /// Speaker notes: the notes slide's body placeholder text, paragraphs
    /// joined with newlines. `None` when the slide has no notes part or the
    /// notes are empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Theme values resolved along this slide’s layout/master relationship chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<std::sync::Arc<SlideTheme>>,
}

/// Reads a presentation and parses every slide body.
///
/// Picture relationship ids are resolved to part names here, and the theme is
/// found through the master's relationships (presentation → slideMaster →
/// theme). A deck missing any of those parts still reads; the affected fields
/// come back empty rather than failing the whole file.
pub fn read_deck(bytes: &[u8]) -> Result<Deck> {
    let package = OpcPackage::open(bytes)?;
    let presentation = read_pptx(bytes)?;

    let (
        theme_colors,
        theme_fonts,
        theme_fill_styles,
        theme_background_fill_styles,
        theme_line_styles,
    ) = read_theme(&package)?;
    let fallback_theme = std::sync::Arc::new(SlideTheme {
        colors: theme_colors.clone(),
        fonts: theme_fonts.clone(),
        fill_styles: theme_fill_styles.clone(),
        background_fill_styles: theme_background_fill_styles.clone(),
        line_styles: theme_line_styles.clone(),
        effect_styles: Vec::new(),
    });
    let mut themes = slide_theme::ThemeResolver::default();
    let mut slides = Vec::new();
    // Layouts are shared between slides and masters between layouts; parse
    // each part once.
    let mut parsed_parts: std::collections::HashMap<String, Option<SlideContent>> =
        std::collections::HashMap::new();
    for entry in &presentation.slides {
        let xml = package.required_part_str(&entry.part_name)?;
        let mut content = parse_slide_xml(&xml)?;
        let mut layout_part = None;
        let mut notes = None;
        if let Some(rels_xml) = package.part_str(&jade_opc::rels_part_name(&entry.part_name))? {
            let relationships = jade_opc::parse_relationships(&rels_xml)?;
            resolve_content_images(&mut content, &relationships, &entry.part_name);
            resolve_hyperlinks(&mut content.shapes, &relationships);
            resolve_charts(
                &package,
                &mut content.shapes,
                &relationships,
                &entry.part_name,
            )?;
            resolve_diagrams(
                &package,
                &mut content.shapes,
                &relationships,
                &entry.part_name,
            )?;
            layout_part = relationships
                .by_type_suffix("/slideLayout")
                .into_iter()
                .find(|r| !r.external)
                .map(|r| jade_opc::resolve_target(&entry.part_name, &r.target));
            if let Some(target) = relationships.target_by_type_suffix("/notesSlide") {
                let part = jade_opc::resolve_target(&entry.part_name, target);
                if let Some(notes_xml) = package.part_str(&part)? {
                    notes = notes_text(&parse_slide_xml(&notes_xml)?);
                }
            }
        }
        resolve_slide_number_fields(&mut content.shapes, slides.len() + 1);
        let layout_content = match &layout_part {
            Some(part) => parsed_shape_tree(&package, part, &mut parsed_parts)?,
            None => None,
        };
        let master_part = match &layout_part {
            Some(part) => match package.part_str(&jade_opc::rels_part_name(part))? {
                Some(rels_xml) => jade_opc::parse_relationships(&rels_xml)?
                    .by_type_suffix("/slideMaster")
                    .into_iter()
                    .find(|r| !r.external)
                    .map(|r| jade_opc::resolve_target(part, &r.target)),
                None => None,
            },
            None => None,
        };
        let master_content = match &master_part {
            Some(part) => parsed_shape_tree(&package, part, &mut parsed_parts)?,
            None => None,
        };
        slides.push(DeckSlide {
            id: entry.id,
            part_name: entry.part_name.clone(),
            content,
            layout_content,
            master_content,
            notes,
            theme: Some(themes.resolve(
                &package,
                master_part.as_deref(),
                layout_part.as_deref(),
                &entry.part_name,
                &fallback_theme,
            )?),
        });
    }

    let presentation_part = presentation_part_name(&package)?;
    let default_text_style = match package.part_str(&presentation_part)? {
        Some(xml) => parse_default_text_style(&xml)?,
        None => ListStyle::default(),
    };
    let table_styles = match package.part_str(&jade_opc::rels_part_name(&presentation_part))? {
        Some(rels_xml) => match jade_opc::parse_relationships(&rels_xml)?
            .target_by_type_suffix("/tableStyles")
            .map(|target| jade_opc::resolve_target(&presentation_part, target))
            .and_then(|part| package.part_str(&part).ok().flatten())
        {
            Some(xml) => parse_table_styles(&xml)?,
            None => Vec::new(),
        },
        None => Vec::new(),
    };

    Ok(Deck {
        slide_size: presentation.slide_size,
        slides,
        theme_colors,
        theme_fonts,
        default_text_style,
        table_styles,
        theme_fill_styles,
        theme_background_fill_styles,
        theme_line_styles,
    })
}

/// A `slidenum` field draws the slide's real number, not whatever number
/// was cached when the deck was last saved (slides get reordered).
fn resolve_slide_number_fields(shapes: &mut [ShapeItem], slide_number: usize) {
    fn resolve_body(body: &mut TextBody, slide_number: usize) {
        for paragraph in &mut body.paragraphs {
            for run in &mut paragraph.runs {
                if run.field.as_deref() == Some("slidenum") {
                    run.text = slide_number.to_string();
                }
            }
        }
    }
    for item in shapes {
        match item {
            ShapeItem::Shape(shape) => {
                if let Some(text) = &mut shape.text {
                    resolve_body(text, slide_number);
                }
            }
            ShapeItem::Group(group) => {
                resolve_slide_number_fields(&mut group.children, slide_number)
            }
            ShapeItem::Table(table) => {
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        if let Some(text) = &mut cell.text {
                            resolve_body(text, slide_number);
                        }
                    }
                }
            }
            ShapeItem::Picture(_) | ShapeItem::Chart(_) => {}
        }
    }
}

/// The notes placeholder's text: the `body` placeholder of a notes slide,
/// never the slide-image or slide-number placeholders beside it.
fn notes_text(notes: &SlideContent) -> Option<String> {
    let body = notes.shapes.iter().find_map(|item| match item {
        ShapeItem::Shape(shape)
            if shape
                .placeholder
                .as_ref()
                .is_some_and(|ph| ph.kind == "body") =>
        {
            shape.text.as_ref()
        }
        _ => None,
    })?;
    let text = body
        .paragraphs
        .iter()
        .map(|paragraph| {
            paragraph
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn resolve_hyperlinks(shapes: &mut [ShapeItem], relationships: &jade_opc::Relationships) {
    fn resolve_body(body: &mut TextBody, relationships: &jade_opc::Relationships) {
        for paragraph in &mut body.paragraphs {
            for run in &mut paragraph.runs {
                if let Some(link) = &mut run.hyperlink {
                    link.target = relationships.target(&link.rel_id).map(str::to_string);
                }
            }
        }
    }
    for item in shapes {
        match item {
            ShapeItem::Shape(shape) => {
                if let Some(text) = &mut shape.text {
                    resolve_body(text, relationships);
                }
            }
            ShapeItem::Group(group) => resolve_hyperlinks(&mut group.children, relationships),
            ShapeItem::Table(table) => {
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        if let Some(text) = &mut cell.text {
                            resolve_body(text, relationships);
                        }
                    }
                }
            }
            ShapeItem::Picture(_) | ShapeItem::Chart(_) => {}
        }
    }
}

/// Parses `part`'s `cSld/spTree` once, remembering the result — including a
/// missing part — so shared layouts and masters are not reparsed per slide.
fn parsed_shape_tree(
    package: &OpcPackage,
    part: &str,
    cache: &mut std::collections::HashMap<String, Option<SlideContent>>,
) -> Result<Option<SlideContent>> {
    if let Some(cached) = cache.get(part) {
        return Ok(cached.clone());
    }
    let parsed = match package.part_str(part)? {
        Some(xml) => {
            let mut content = parse_slide_xml(&xml)?;
            // Layout and master pictures (logos, backgrounds) resolve
            // against their own part's relationships.
            if let Some(rels_xml) = package.part_str(&jade_opc::rels_part_name(part))? {
                let relationships = jade_opc::parse_relationships(&rels_xml)?;
                resolve_content_images(&mut content, &relationships, part);
                resolve_hyperlinks(&mut content.shapes, &relationships);
            }
            Some(content)
        }
        None => None,
    };
    cache.insert(part.to_string(), parsed.clone());
    Ok(parsed)
}

/// Resolves a part's background picture and its shapes' pictures.
fn resolve_content_images(
    content: &mut SlideContent,
    relationships: &jade_opc::Relationships,
    part: &str,
) {
    if let Some(Fill::Picture { rel_id, image_part }) = &mut content.background {
        *image_part = relationships
            .target(rel_id)
            .map(|target| jade_opc::resolve_target(part, target));
    }
    resolve_image_parts(&mut content.shapes, relationships, part);
}

fn resolve_image_parts(
    shapes: &mut [ShapeItem],
    relationships: &jade_opc::Relationships,
    slide_part: &str,
) {
    for item in shapes {
        match item {
            ShapeItem::Picture(picture) => {
                picture.image_part = picture
                    .blip_rel_id
                    .as_deref()
                    .and_then(|id| relationships.target(id))
                    .map(|target| jade_opc::resolve_target(slide_part, target));
            }
            ShapeItem::Group(group) => {
                resolve_image_parts(&mut group.children, relationships, slide_part);
            }
            ShapeItem::Shape(shape) => {
                shape.image_part = shape
                    .image_rel_id
                    .as_deref()
                    .and_then(|id| relationships.target(id))
                    .map(|target| jade_opc::resolve_target(slide_part, target));
            }
            ShapeItem::Table(_) | ShapeItem::Chart(_) => {}
        }
    }
}

/// Reads each chart frame's part: kind, title and cached series.
fn resolve_charts(
    package: &OpcPackage,
    shapes: &mut [ShapeItem],
    relationships: &jade_opc::Relationships,
    slide_part: &str,
) -> Result<()> {
    for item in shapes {
        match item {
            ShapeItem::Chart(chart) => {
                let Some(part) = chart
                    .chart_rel_id
                    .as_deref()
                    .and_then(|id| relationships.target(id))
                    .map(|target| jade_opc::resolve_target(slide_part, target))
                else {
                    continue;
                };
                if let Some(xml) = package.part_str(&part)? {
                    let content = parse_chart_xml(&xml)?;
                    chart.kind = content.kind;
                    chart.title = content.title;
                    chart.horizontal = content.horizontal;
                    chart.series = content.series;
                    chart.style = content.style;
                }
                chart.chart_part = Some(part);
            }
            ShapeItem::Group(group) => {
                resolve_charts(package, &mut group.children, relationships, slide_part)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Fills each SmartArt frame with the shapes of the drawing PowerPoint
/// caches next to its data: the data part's `dataModelExt` names the
/// drawing relationship; a slide with a single drawing relationship needs
/// no such link.
fn resolve_diagrams(
    package: &OpcPackage,
    shapes: &mut [ShapeItem],
    relationships: &jade_opc::Relationships,
    slide_part: &str,
) -> Result<()> {
    const DIAGRAM_DRAWING: &str = "/diagramDrawing";
    for item in shapes {
        let ShapeItem::Group(group) = item else {
            continue;
        };
        let Some(data_rel) = group.diagram_data_rel_id.as_deref() else {
            resolve_diagrams(package, &mut group.children, relationships, slide_part)?;
            continue;
        };
        let data_part = relationships
            .target(data_rel)
            .map(|target| jade_opc::resolve_target(slide_part, target));
        let drawing_rel = match data_part.and_then(|part| package.part_str(&part).ok().flatten()) {
            Some(xml) => diagram_drawing_rel_id(&xml)?,
            None => None,
        };
        let drawing_target = drawing_rel
            .as_deref()
            .and_then(|id| relationships.target(id))
            .or_else(|| relationships.target_by_type_suffix(DIAGRAM_DRAWING));
        let Some(drawing_part) =
            drawing_target.map(|target| jade_opc::resolve_target(slide_part, target))
        else {
            continue;
        };
        if let Some(xml) = package.part_str(&drawing_part)? {
            let mut children = parse_diagram_drawing_xml(&xml)?;
            resolve_image_parts(&mut children, relationships, slide_part);
            group.children = children;
        }
    }
    Ok(())
}

fn presentation_part_name(package: &OpcPackage) -> Result<String> {
    Ok(match package.part_str("_rels/.rels")? {
        Some(xml) => jade_opc::parse_relationships(&xml)?
            .target_by_type_suffix(OFFICE_DOCUMENT)
            .map(|target| jade_opc::resolve_target("", target))
            .unwrap_or_else(|| "ppt/presentation.xml".to_string()),
        None => "ppt/presentation.xml".to_string(),
    })
}

/// Follows presentation → slideMaster → theme. Every part along the way is
/// optional: a deck without a readable theme keeps an empty scheme. The
/// master's colour map rides along, since it is what turns `bg1` into a
/// scheme slot.
type ThemeParts = (ThemeColors, ThemeFonts, Vec<Fill>, Vec<Fill>, Vec<Outline>);

fn read_theme(package: &OpcPackage) -> Result<ThemeParts> {
    const SLIDE_MASTER: &str = "/slideMaster";

    let presentation_part = presentation_part_name(package)?;

    let master_part = match package.part_str(&jade_opc::rels_part_name(&presentation_part))? {
        Some(xml) => jade_opc::parse_relationships(&xml)?
            .target_by_type_suffix(SLIDE_MASTER)
            .map(|target| jade_opc::resolve_target(&presentation_part, target)),
        None => None,
    };

    read_master_theme(package, master_part.as_deref())
}
/// The effect styles of the theme a master uses.
pub(crate) fn read_master_effect_styles(
    package: &OpcPackage,
    master_part: Option<&str>,
) -> Result<Vec<Option<Shadow>>> {
    let Some(master) = master_part else {
        return Ok(Vec::new());
    };
    let theme_part = match package.part_str(&jade_opc::rels_part_name(master))? {
        Some(xml) => jade_opc::parse_relationships(&xml)?
            .by_type_suffix("/theme")
            .into_iter()
            .find(|r| !r.external)
            .map(|r| jade_opc::resolve_target(master, &r.target)),
        None => None,
    };
    match theme_part.and_then(|part| package.part_str(&part).ok().flatten()) {
        Some(xml) => parse_theme_effect_styles(&xml),
        None => Ok(Vec::new()),
    }
}

fn read_master_theme(package: &OpcPackage, master_part: Option<&str>) -> Result<ThemeParts> {
    let color_map = match master_part {
        Some(master) => match package.part_str(master)? {
            Some(xml) => parse_color_map(&xml)?,
            None => Default::default(),
        },
        None => Default::default(),
    };

    let theme_part = match master_part {
        Some(master) => match package.part_str(&jade_opc::rels_part_name(master))? {
            Some(xml) => jade_opc::parse_relationships(&xml)?
                .by_type_suffix("/theme")
                .into_iter()
                .find(|r| !r.external)
                .map(|r| jade_opc::resolve_target(master, &r.target)),
            None => None,
        },
        None => None,
    };

    let (mut colors, fonts, fill_styles, background_fill_styles, line_styles) =
        match theme_part.and_then(|part| package.part_str(&part).ok().flatten()) {
            Some(xml) => {
                let (fills, background_fills) = parse_theme_fill_styles(&xml)?;
                (
                    parse_theme_colors(&xml)?,
                    parse_theme_fonts(&xml)?,
                    fills,
                    background_fills,
                    parse_theme_line_styles(&xml)?,
                )
            }
            None => (
                ThemeColors::default(),
                ThemeFonts::default(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
        };
    colors.set_color_map(color_map);
    Ok((
        colors,
        fonts,
        fill_styles,
        background_fill_styles,
        line_styles,
    ))
}

/// A `<p:sldId>` entry before its relationship is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SlideId {
    id: Option<u32>,
    relationship_id: Option<String>,
}

fn parse_presentation(xml: &str) -> Result<(SlideSize, Vec<SlideId>)> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut slide_size = SlideSize::default();
    let mut slides = Vec::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(ref element) | Event::Empty(ref element) => {
                match local_name(element.name().as_ref()) {
                    b"sldSz" => {
                        for attribute in element.attributes() {
                            let attribute = attribute?;
                            let value = attribute.unescape_value()?;
                            match local_name(attribute.key.as_ref()) {
                                b"cx" => {
                                    if let Ok(width) = value.parse() {
                                        slide_size.width_emu = width;
                                    }
                                }
                                b"cy" => {
                                    if let Ok(height) = value.parse() {
                                        slide_size.height_emu = height;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    b"sldId" => {
                        let mut id = None;
                        let mut relationship_id = None;
                        for attribute in element.attributes() {
                            let attribute = attribute?;
                            let key = attribute.key.as_ref();
                            let value = attribute.unescape_value()?;
                            // `r:id` and the plain `id` attribute share a local
                            // name, so the namespace prefix is what tells them
                            // apart.
                            if key == b"id" {
                                id = value.parse().ok();
                            } else if local_name(key) == b"id" {
                                relationship_id = Some(value.into_owned());
                            }
                        }
                        slides.push(SlideId {
                            id,
                            relationship_id,
                        });
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok((slide_size, slides))
}
