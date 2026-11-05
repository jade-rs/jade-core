//! 原稿用紙設定 (レイアウト › 原稿用紙設定): the manuscript-paper grid Word
//! draws behind the text — squares (マス目付き), underlines (下線付き) or a
//! frame with ruled lines (外枠付き). Word keeps it as a drawing in every
//! header — a VML group whose id starts `Genko:` (paper, characters, lines
//! and orientation) — over a section whose `w:docGrid` is `snapToChars`,
//! one character to a square. The grid is written here as such a group of
//! rectangles and lines, read back from it, and its geometry is shared with
//! the renderer so the squares drawn are the ones saved.

use jade_docx_reader::document::{ManuscriptGrid, ManuscriptStyle, TextDirection};

use super::*;

/// One stroke of the grid, in points from the body box's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ManuscriptStroke {
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    Line {
        from: (f32, f32),
        to: (f32, f32),
    },
}

/// Whether a section's text direction runs its lines vertically (縦書き).
pub fn manuscript_is_vertical(direction: Option<TextDirection>) -> bool {
    matches!(
        direction,
        Some(TextDirection::TbRl | TextDirection::TbRlV | TextDirection::TbLrV)
    )
}

/// The strokes of `grid` over a body box of `width` × `height` points.
/// Horizontal text runs `chars` squares across and `lines` rows down;
/// vertical text (縦書き) runs `chars` squares down and `lines` columns from
/// the right. `pitches` is the section grid's character and line pitch (in
/// the same unit as the box): the text is laid one character to a cell of
/// those pitches, centred in it, so the squares take the same pitches and
/// each sits in the middle of its cell, under its character. A pitch that
/// would not fit its count in the box shares the box out evenly instead.
pub fn manuscript_strokes(
    grid: &ManuscriptGrid,
    vertical: bool,
    width: f32,
    height: f32,
    pitches: Option<(f32, f32)>,
) -> Vec<ManuscriptStroke> {
    let chars = grid.chars.max(1) as f32;
    let lines = grid.lines.max(1) as f32;
    let mut strokes = Vec::new();
    if width <= 0.0 || height <= 0.0 {
        return strokes;
    }
    // Along the line (a character's pitch) and across it (the line pitch).
    let (along, across) = if vertical {
        (height, width)
    } else {
        (width, height)
    };
    let pitch = |pitch: Option<f32>, count: f32, extent: f32| {
        pitch
            .filter(|pitch| pitch.is_finite() && *pitch > 0.0 && pitch * count <= extent * 1.001)
            .unwrap_or(extent / count)
    };
    let char_pitch = pitch(pitches.map(|pitches| pitches.0), chars, along);
    let line_pitch = pitch(pitches.map(|pitches| pitches.1), lines, across);
    let side = char_pitch.min(line_pitch);
    let inset = (line_pitch - side) / 2.0;
    // The square of character `c` on line `l`.
    let square = |l: u32, c: u32| -> (f32, f32) {
        if vertical {
            (
                width - (l as f32 + 1.0) * line_pitch + inset,
                c as f32 * char_pitch + (char_pitch - side) / 2.0,
            )
        } else {
            (
                c as f32 * char_pitch + (char_pitch - side) / 2.0,
                l as f32 * line_pitch + inset,
            )
        }
    };
    match grid.style {
        ManuscriptStyle::Squares => {
            strokes.push(ManuscriptStroke::Rect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            });
            for l in 0..grid.lines {
                for c in 0..grid.chars {
                    let (x, y) = square(l, c);
                    strokes.push(ManuscriptStroke::Rect {
                        x,
                        y,
                        width: side,
                        height: side,
                    });
                }
            }
        }
        ManuscriptStyle::Lines => {
            for l in 0..grid.lines {
                let (x, y) = square(l, 0);
                strokes.push(if vertical {
                    // Beside the column, where a vertical line's underline
                    // falls.
                    ManuscriptStroke::Line {
                        from: (x + side, 0.0),
                        to: (x + side, height),
                    }
                } else {
                    ManuscriptStroke::Line {
                        from: (0.0, y + side),
                        to: (width, y + side),
                    }
                });
            }
        }
        ManuscriptStyle::Frame => {
            strokes.push(ManuscriptStroke::Rect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            });
            for l in 1..grid.lines {
                let at = l as f32 * line_pitch;
                strokes.push(if vertical {
                    ManuscriptStroke::Line {
                        from: (width - at, 0.0),
                        to: (width - at, height),
                    }
                } else {
                    ManuscriptStroke::Line {
                        from: (0.0, at),
                        to: (width, at),
                    }
                });
            }
        }
    }
    strokes
}

/// The paper's name in the group's id, as Word's 原稿用紙 names them.
fn paper_name(width: f32, height: f32) -> &'static str {
    let (short, long) = if width <= height {
        (width, height)
    } else {
        (height, width)
    };
    let near = |a: f32, b: f32| (a - b).abs() < 3.0;
    if near(short, 595.3) && near(long, 841.9) {
        "A4"
    } else if near(short, 515.9) && near(long, 728.5) {
        "B5"
    } else if near(short, 841.9) && near(long, 1190.55) {
        "A3"
    } else if near(short, 728.5) && near(long, 1031.8) {
        "B4"
    } else if near(short, 419.55) && near(long, 595.3) {
        "A5"
    } else if near(short, 612.0) && near(long, 792.0) {
        "Letter"
    } else {
        "Custom"
    }
}

/// The body box of the final section, in points, and whether its text runs
/// vertically.
struct BodyBox {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
    page_width: f32,
    page_height: f32,
    vertical: bool,
    /// The section grid's character and line pitch, when it has both.
    pitches: Option<(f32, f32)>,
}

impl WordDocument {
    /// Sets or takes away the manuscript-paper grid in every header, as
    /// 原稿用紙設定 does, over the final section's page and margins (which
    /// the caller sets first, with its `w:docGrid`). Returns `false` for an
    /// exact no-op.
    pub fn set_manuscript_grid(&mut self, grid: Option<ManuscriptGrid>) -> Result<bool, String> {
        if self.document.is_none() || self.raw_document.is_none() {
            return Err("document is unavailable".to_string());
        }
        let grid = match grid {
            Some(mut grid) => {
                if !(1..=200).contains(&grid.chars) || !(1..=200).contains(&grid.lines) {
                    return Err(
                        "manuscript grid needs 1 to 200 characters and 1 to 200 lines".to_string(),
                    );
                }
                grid.color = normalize_watermark_color(&grid.color).ok_or_else(|| {
                    format!("manuscript color must be #RRGGBB, not {}", grid.color)
                })?;
                Some(grid)
            }
            None => None,
        };
        let body = self.manuscript_body_box();
        let variants = [
            HeaderFooterVariant::Default,
            HeaderFooterVariant::First,
            HeaderFooterVariant::Even,
        ];
        if grid.is_none() {
            let any = variants.into_iter().any(|variant| {
                self.document
                    .as_deref()
                    .and_then(|document| {
                        pick_hf_reference(document, HeaderFooterRole::Header, variant)
                    })
                    .is_some()
                    && self
                        .resolve_header_footer_part_path_for(HeaderFooterRole::Header, variant)
                        .ok()
                        .and_then(|path| self.read_xml_part_content(&path))
                        .is_some_and(|xml| xml.contains("Genko:"))
            });
            if !any {
                if let Some(document) = self.document_mut() {
                    document.manuscript_grid = None;
                }
                return Ok(false);
            }
        } else {
            self.ensure_hf_part_for_edit(HeaderFooterRole::Header, HeaderFooterVariant::Default)?;
        }
        for variant in variants {
            let present = self
                .document
                .as_deref()
                .and_then(|document| pick_hf_reference(document, HeaderFooterRole::Header, variant))
                .is_some();
            if present {
                self.sync_manuscript_to_header_variant(variant, grid.as_ref(), &body)?;
            }
        }
        if let Some(document) = self.document_mut() {
            document.manuscript_grid = grid;
        }
        Ok(true)
    }

    fn manuscript_body_box(&self) -> BodyBox {
        let page = self.section_page_settings(None).unwrap_or_default();
        let page_width = page.width.unwrap_or(595.3);
        let page_height = page.height.unwrap_or(841.9);
        let left = page.margin_left.unwrap_or(85.05);
        let right = page.margin_right.unwrap_or(85.05);
        let top = page.margin_top.unwrap_or(99.25);
        let bottom = page.margin_bottom.unwrap_or(85.05);
        let direction = self.document.as_deref().and_then(|document| {
            document
                .sections
                .last()
                .and_then(|section| section.properties.as_ref())
                .and_then(|properties| properties.text_direction)
                .or_else(|| {
                    document
                        .default_section
                        .as_ref()
                        .and_then(|properties| properties.text_direction)
                })
        });
        let grid = self.section_layout(None).doc_grid;
        BodyBox {
            left,
            top,
            width: (page_width - left - right).max(1.0),
            height: (page_height - top - bottom).max(1.0),
            page_width,
            page_height,
            vertical: manuscript_is_vertical(direction),
            pitches: grid.char_pitch.zip(grid.line_pitch),
        }
    }

    /// The grid an opened document's headers draw, if any.
    pub(crate) fn manuscript_from_loaded_headers(
        document: &WordDocument,
        parsed_document: &Document,
        document_part: &DocumentPart,
    ) -> Option<ManuscriptGrid> {
        for variant in [
            HeaderFooterVariant::Default,
            HeaderFooterVariant::First,
            HeaderFooterVariant::Even,
        ] {
            let Some(reference_id) =
                pick_hf_reference(parsed_document, HeaderFooterRole::Header, variant)
            else {
                continue;
            };
            let Some(relationship) = document_part
                .relationships
                .iter()
                .find(|relationship| relationship.id == reference_id)
            else {
                continue;
            };
            let path = Self::resolve_target_path(&document_part.path, &relationship.target);
            let Some(content) = document
                .parts
                .get(&path)
                .map(|part| part.content.as_str())
                .or_else(|| {
                    document
                        .package
                        .as_ref()
                        .and_then(|package| package.get(&path))
                        .and_then(|bytes| std::str::from_utf8(bytes).ok())
                })
            else {
                continue;
            };
            if let Some(grid) = Self::manuscript_from_xml(content) {
                return Some(grid);
            }
        }
        None
    }

    /// The manuscript grid a header's `Genko:` group draws, if it has one.
    pub(crate) fn manuscript_from_xml(xml: &str) -> Option<ManuscriptGrid> {
        if !xml.contains("Genko:") {
            return None;
        }
        let document = DocxXmlDocument::parse(xml).ok()?;
        let group = Self::manuscript_group(document.root())?;
        let id = group
            .attributes
            .iter()
            .find(|attribute| attribute.name.rsplit(':').next() == Some("id"))
            .map(|attribute| attribute.value.clone())?;
        let mut fields = id.split(':').skip(2);
        let chars = fields.next()?.parse::<u32>().ok()?;
        let lines = fields.next()?.parse::<u32>().ok()?;
        let (mut rects, mut color) = (0usize, None);
        for child in &group.children {
            let XmlNode::Element(child) = child else {
                continue;
            };
            match child.name.rsplit(':').next() {
                Some("rect") => rects += 1,
                Some("line") => {}
                _ => continue,
            }
            if color.is_none() {
                color = child
                    .attributes
                    .iter()
                    .find(|attribute| attribute.name.rsplit(':').next() == Some("strokecolor"))
                    .and_then(|attribute| normalize_watermark_color(&attribute.value));
            }
        }
        let style = if rects > 1 {
            ManuscriptStyle::Squares
        } else if rects == 1 {
            ManuscriptStyle::Frame
        } else {
            ManuscriptStyle::Lines
        };
        Some(ManuscriptGrid {
            style,
            chars,
            lines,
            color: color.unwrap_or_else(|| "#000000".to_string()),
        })
    }

    fn manuscript_group(element: &XmlElement) -> Option<&XmlElement> {
        if element.attributes.iter().any(|attribute| {
            attribute.name.rsplit(':').next() == Some("id") && attribute.value.starts_with("Genko:")
        }) {
            return Some(element);
        }
        element.children.iter().find_map(|child| match child {
            XmlNode::Element(child) => Self::manuscript_group(child),
            _ => None,
        })
    }

    fn remove_manuscript_runs(element: &mut XmlElement) {
        let children = std::mem::take(&mut element.children);
        for child in children {
            match child {
                XmlNode::Element(mut child) => {
                    if child.name == "w:r" && Self::manuscript_group(&child).is_some() {
                        continue;
                    }
                    Self::remove_manuscript_runs(&mut child);
                    element.children.push(XmlNode::Element(child));
                }
                other => element.children.push(other),
            }
        }
    }

    fn sync_manuscript_to_header_variant(
        &mut self,
        variant: HeaderFooterVariant,
        grid: Option<&ManuscriptGrid>,
        body: &BodyBox,
    ) -> Result<(), String> {
        let part_path =
            self.resolve_header_footer_part_path_for(HeaderFooterRole::Header, variant)?;
        let existing = self
            .read_xml_part_content(&part_path)
            .ok_or_else(|| format!("missing {part_path}"))?;
        let mut xml = DocxXmlDocument::parse(&existing).map_err(|error| error.to_string())?;
        let root_id = xml.root().id;
        {
            let root = xml
                .element_mut(root_id)
                .ok_or_else(|| format!("missing root element in {part_path}"))?;
            Self::remove_manuscript_runs(root);
            if grid.is_some() {
                for (name, value) in [
                    ("xmlns:v", "urn:schemas-microsoft-com:vml"),
                    ("xmlns:o", "urn:schemas-microsoft-com:office:office"),
                ] {
                    if !root
                        .attributes
                        .iter()
                        .any(|attribute| attribute.name == name)
                    {
                        root.attributes.push(XmlAttribute {
                            name: name.to_string(),
                            value: value.to_string(),
                        });
                    }
                }
            }
        }
        xml.rebuild_index();
        if let Some(grid) = grid {
            let paragraph_id = xml.root().children.iter().find_map(|child| match child {
                XmlNode::Element(element) if element.name == "w:p" => Some(element.id),
                _ => None,
            });
            let paragraph_id = match paragraph_id {
                Some(id) => id,
                None => {
                    let paragraph = xml.create_element("w:p", Vec::new(), Vec::new());
                    let id = paragraph.id;
                    xml.insert_node(root_id, 0, XmlNode::Element(paragraph))
                        .map_err(|error| error.to_string())?;
                    id
                }
            };
            let run = Self::build_manuscript_run(&mut xml, grid, body);
            let insert_at = xml
                .element(paragraph_id)
                .map(|paragraph| {
                    paragraph
                        .children
                        .iter()
                        .position(|child| {
                            !matches!(child, XmlNode::Element(element) if element.name == "w:pPr")
                        })
                        .unwrap_or(paragraph.children.len())
                })
                .unwrap_or(0);
            xml.insert_node(paragraph_id, insert_at, XmlNode::Element(run))
                .map_err(|error| error.to_string())?;
        }
        let serialized = xml.to_string().map_err(|error| error.to_string())?;
        self.upsert_xml_part_content(&part_path, serialized);
        self.refresh_header_footer_lines_for(HeaderFooterRole::Header, variant, &part_path)
    }

    /// `w:r/w:pict/v:group`: the grid as Word draws it, a group over the
    /// body box in twips with one shape per square or line.
    fn build_manuscript_run(
        xml: &mut DocxXmlDocument,
        grid: &ManuscriptGrid,
        body: &BodyBox,
    ) -> XmlElement {
        fn attr(name: &str, value: impl Into<String>) -> XmlAttribute {
            XmlAttribute {
                name: name.to_string(),
                value: value.into(),
            }
        }
        let twips = |points: f32| (points * 20.0).round() as i64;
        let color = grid.color.clone();
        let mut shapes = Vec::new();
        for stroke in manuscript_strokes(grid, body.vertical, body.width, body.height, body.pitches)
        {
            let element = match stroke {
                ManuscriptStroke::Rect {
                    x,
                    y,
                    width,
                    height,
                } => xml.create_element(
                    "v:rect",
                    vec![
                        attr(
                            "style",
                            format!(
                                "position:absolute;left:{};top:{};width:{};height:{}",
                                twips(x),
                                twips(y),
                                twips(width),
                                twips(height)
                            ),
                        ),
                        attr("filled", "f"),
                        attr("strokecolor", color.clone()),
                        attr("strokeweight", ".5pt"),
                    ],
                    Vec::new(),
                ),
                ManuscriptStroke::Line { from, to } => xml.create_element(
                    "v:line",
                    vec![
                        attr("style", "position:absolute"),
                        attr("from", format!("{},{}", twips(from.0), twips(from.1))),
                        attr("to", format!("{},{}", twips(to.0), twips(to.1))),
                        attr("strokecolor", color.clone()),
                        attr("strokeweight", ".5pt"),
                    ],
                    Vec::new(),
                ),
            };
            shapes.push(XmlNode::Element(element));
        }
        let orientation = if body.page_width > body.page_height {
            "L"
        } else {
            "P"
        };
        let id = format!(
            "Genko:{}:{}:{}:{}:0::",
            paper_name(body.page_width, body.page_height),
            grid.chars,
            grid.lines,
            orientation
        );
        let group = xml.create_element(
            "v:group",
            vec![
                attr("id", id),
                attr(
                    "style",
                    format!(
                        "position:absolute;left:0;text-align:left;margin-left:{}pt;margin-top:{}pt;width:{}pt;height:{}pt;z-index:-251657216;mso-position-horizontal-relative:page;mso-position-vertical-relative:page",
                        trim_watermark_number(body.left),
                        trim_watermark_number(body.top),
                        trim_watermark_number(body.width),
                        trim_watermark_number(body.height)
                    ),
                ),
                attr("coordorigin", "0,0"),
                attr(
                    "coordsize",
                    format!("{},{}", twips(body.width), twips(body.height)),
                ),
                attr("o:allowincell", "f"),
            ],
            shapes,
        );
        let pict = xml.create_element("w:pict", Vec::new(), vec![XmlNode::Element(group)]);
        let no_proof = xml.create_element("w:noProof", Vec::new(), Vec::new());
        let properties = xml.create_element("w:rPr", Vec::new(), vec![XmlNode::Element(no_proof)]);
        xml.create_element(
            "w:r",
            Vec::new(),
            vec![XmlNode::Element(properties), XmlNode::Element(pict)],
        )
    }
}
