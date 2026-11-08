//! Notes pages and handouts: ノート マスター (`notesMaster`), 配布資料マスター
//! (`handoutMaster`), each slide's notes slide and the page size they share
//! (`<p:notesSz>`).
//!
//! A notes page is laid out like a slide — the notes slide's shapes over the
//! notes master's — at the notes page size; the `sldImg` placeholder is where
//! the slide itself is drawn. A handout page is the handout master's shapes
//! with the slides arranged by how many go on a page.

use std::sync::Arc;

use jade_opc::OpcPackage;
use serde::{Deserialize, Serialize};

use crate::xml_tree::parse_tree;
use crate::{
    parse_slide_xml, Result, ShapeItem, SlideContent, SlideSize, SlideTheme, TextStyles, Transform,
};

/// `<p:hf>`: which header and footer placeholders a master shows. Each
/// defaults to shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderFooterFlags {
    pub header: bool,
    pub footer: bool,
    pub date: bool,
    pub slide_number: bool,
}

impl Default for HeaderFooterFlags {
    fn default() -> Self {
        Self {
            header: true,
            footer: true,
            date: true,
            slide_number: true,
        }
    }
}

/// A notes or handout master, read for drawing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageMaster {
    pub part_name: String,
    /// Its shape tree; on a notes master `text_styles.body` is its
    /// `<p:notesStyle>`, which the notes text inherits.
    pub content: SlideContent,
    pub header_footer: HeaderFooterFlags,
    pub theme: Arc<SlideTheme>,
}

/// One slide's notes slide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotesSlide {
    pub part_name: String,
    pub content: SlideContent,
}

/// Everything needed to draw notes pages and handouts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotesPages {
    /// `<p:notesSz>`: notes pages and handouts share it. Portrait unless
    /// 配布資料の向き / ノートの向き turned it.
    pub page_size: SlideSize,
    pub notes_master: Option<PageMaster>,
    pub handout_master: Option<PageMaster>,
    /// One per slide; `None` for a slide without notes.
    pub notes: Vec<Option<NotesSlide>>,
}

impl NotesPages {
    /// Where the slide image sits on slide `index`'s notes page: the notes
    /// slide's `sldImg` placeholder, else the notes master's, else
    /// PowerPoint's default place for it.
    pub fn slide_image_rect(&self, index: usize) -> Transform {
        let own = self
            .notes
            .get(index)
            .and_then(Option::as_ref)
            .and_then(|notes| placeholder_transform(&notes.content, "sldImg"));
        own.or_else(|| {
            self.notes_master
                .as_ref()
                .and_then(|master| placeholder_transform(&master.content, "sldImg"))
        })
        .unwrap_or_else(|| default_slide_image(self.page_size))
    }
}

/// A placeholder's own transform, by `type`.
pub fn placeholder_transform(content: &SlideContent, kind: &str) -> Option<Transform> {
    content.shapes.iter().find_map(|item| match item {
        ShapeItem::Shape(shape) if shape.placeholder.as_ref().is_some_and(|ph| ph.kind == kind) => {
            shape.transform
        }
        _ => None,
    })
}

/// The slide image on a notes page without a notes master: the top half,
/// centred, a little in from the edges — where PowerPoint's own default
/// notes master puts it.
pub fn default_slide_image(page: SlideSize) -> Transform {
    let width = page.width_emu * 3 / 4;
    let height = page.height_emu * 3 / 8;
    Transform {
        offset_emu: ((page.width_emu - width) / 2, page.height_emu / 12),
        extent_emu: (width, height),
        rotation_60000ths: 0,
        flip_h: false,
        flip_v: false,
    }
}

/// PowerPoint's notes page: US Letter portrait, 7.5 × 10 in.
pub const DEFAULT_NOTES_SIZE: SlideSize = SlideSize {
    width_emu: 6_858_000,
    height_emu: 9_144_000,
};

fn read_page_master(
    package: &OpcPackage,
    part: &str,
    fallback: &Arc<SlideTheme>,
) -> Result<Option<PageMaster>> {
    let Some(xml) = package.part_str(part)? else {
        return Ok(None);
    };
    let root = parse_tree(&xml)?;
    let mut content = parse_slide_xml(&xml)?;
    if let Some(style) = root.child("notesStyle") {
        content.text_styles = Some(TextStyles {
            body: crate::slide::parse_list_style(style),
            ..TextStyles::default()
        });
    }
    if let Some(rels) = package.part_str(&jade_opc::rels_part_name(part))? {
        let relationships = jade_opc::parse_relationships(&rels)?;
        crate::resolve_content_images(&mut content, &relationships, part);
    }
    let header_footer = root
        .child("hf")
        .map(|hf| {
            let on = |name: &str| !matches!(hf.attr(name), Some("0" | "false"));
            HeaderFooterFlags {
                header: on("hdr"),
                footer: on("ftr"),
                date: on("dt"),
                slide_number: on("sldNum"),
            }
        })
        .unwrap_or_default();
    let theme = if package
        .part_str(&jade_opc::rels_part_name(part))?
        .is_some_and(|rels| rels.contains("/theme\""))
    {
        let (colors, fonts, fill_styles, background_fill_styles, line_styles) =
            crate::read_master_theme(package, Some(part))?;
        Arc::new(SlideTheme {
            colors,
            fonts,
            fill_styles,
            background_fill_styles,
            line_styles,
            effect_styles: Vec::new(),
            effect_style_effects: Vec::new(),
        })
    } else {
        fallback.clone()
    };
    Ok(Some(PageMaster {
        part_name: part.to_string(),
        content,
        header_footer,
        theme,
    }))
}

/// Reads the notes size, both masters and every slide's notes slide.
pub fn read_notes_pages(bytes: &[u8]) -> Result<NotesPages> {
    let package = OpcPackage::open(bytes)?;
    let presentation = crate::presentation_part_name(&package)?;
    let presentation_xml = package.required_part_str(&presentation)?;
    let root = parse_tree(&presentation_xml)?;
    let page_size = root
        .child("notesSz")
        .and_then(|size| {
            Some(SlideSize {
                width_emu: size.attr("cx")?.parse().ok()?,
                height_emu: size.attr("cy")?.parse().ok()?,
            })
        })
        .filter(|size| size.width_emu > 0 && size.height_emu > 0)
        .unwrap_or(DEFAULT_NOTES_SIZE);
    let rels = match package.part_str(&jade_opc::rels_part_name(&presentation))? {
        Some(xml) => jade_opc::parse_relationships(&xml)?,
        None => Default::default(),
    };
    let (colors, fonts, fill_styles, background_fill_styles, line_styles) =
        crate::read_theme(&package)?;
    let fallback = Arc::new(SlideTheme {
        colors,
        fonts,
        fill_styles,
        background_fill_styles,
        line_styles,
        effect_styles: Vec::new(),
        effect_style_effects: Vec::new(),
    });
    let master = |suffix: &str| {
        rels.target_by_type_suffix(suffix)
            .map(|target| jade_opc::resolve_target(&presentation, target))
    };
    let notes_master = match master("/notesMaster") {
        Some(part) => read_page_master(&package, &part, &fallback)?,
        None => None,
    };
    let handout_master = match master("/handoutMaster") {
        Some(part) => read_page_master(&package, &part, &fallback)?,
        None => None,
    };
    let mut notes = Vec::new();
    for slide in crate::read_pptx(bytes)?.slides {
        let part = match package.part_str(&jade_opc::rels_part_name(&slide.part_name))? {
            Some(xml) => jade_opc::parse_relationships(&xml)?
                .target_by_type_suffix("/notesSlide")
                .map(|target| jade_opc::resolve_target(&slide.part_name, target)),
            None => None,
        };
        let notes_slide = match part {
            Some(part) => match package.part_str(&part)? {
                Some(xml) => {
                    let mut content = parse_slide_xml(&xml)?;
                    if let Some(rels) = package.part_str(&jade_opc::rels_part_name(&part))? {
                        let relationships = jade_opc::parse_relationships(&rels)?;
                        crate::resolve_content_images(&mut content, &relationships, &part);
                    }
                    Some(NotesSlide {
                        part_name: part,
                        content,
                    })
                }
                None => None,
            },
            None => None,
        };
        notes.push(notes_slide);
    }
    Ok(NotesPages {
        page_size,
        notes_master,
        handout_master,
        notes,
    })
}
