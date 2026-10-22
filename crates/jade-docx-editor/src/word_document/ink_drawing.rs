//! Word ink as the 描画 tab edits it: each ink drawing is a floating drawing
//! whose graphic is a `w14:contentPart` over an InkML part, inside an
//! `mc:AlternateContent` whose fallback is a PNG of the strokes — what Word
//! writes, and what it and older consumers read.
//!
//! On load the InkML is read into the drawing's [`RunImage::ink`]; the
//! renderer draws those strokes. Editing replaces the drawings wholesale
//! ([`WordDocument::set_ink_drawings`]), keeping the ones asked to stay.

use std::collections::{HashMap, HashSet};

use base64::Engine;
use serde::{Deserialize, Serialize};

use super::{
    DocumentPart, ImagePosition, OpenXmlPackage, Relationship, Run, RunImage, RunImageKind,
    RunImageLayout, WordDocument, WrapMode, BASE64,
};
use crate::docx_xml::{DocxXmlDocument, XmlAttribute, XmlElement, XmlNode};
use jade_docx_reader::inkml::{
    fit_traces_to_box, parse_inkml, write_inkml, InkContent, InkTool, InkTrace, INK_CONTENT_TYPE,
    INK_RELATIONSHIP_TYPE,
};

/// One ink drawing in the body: where it is anchored and what it holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InkDrawingInfo {
    pub paragraph_index: usize,
    pub run_index: usize,
    /// Whether both offsets are from the page's edges, so `x`/`y` are page
    /// coordinates. Otherwise the renderer knows where it landed.
    pub page_relative: bool,
    pub x_px: f64,
    pub y_px: f64,
    pub width_px: f64,
    pub height_px: f64,
    /// Strokes in px within the drawing's box.
    pub traces: Vec<InkTrace>,
}

/// One ink drawing [`WordDocument::set_ink_drawings`] leaves in the body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InkDrawingSpec {
    /// The drawing at this paragraph and run, as it is.
    Keep {
        paragraph_index: usize,
        run_index: usize,
    },
    /// A drawing of `traces` (page px) anchored in `paragraph_index`,
    /// positioned from the page's top-left.
    New {
        paragraph_index: usize,
        traces: Vec<InkTrace>,
    },
}

/// A box: left, top, width, height in px.
pub type InkBox = (f64, f64, f64, f64);

/// The page-coordinate box of `traces` with their brushes, and the traces
/// moved into it.
pub fn traces_in_box(traces: &[InkTrace]) -> Option<(InkBox, Vec<InkTrace>)> {
    let (left, top, right, bottom) = InkContent::bounds(traces)?;
    let moved = traces
        .iter()
        .map(|trace| InkTrace {
            points: trace
                .points
                .iter()
                .map(|&(x, y)| (x - left, y - top))
                .collect(),
            brush: trace.brush.clone(),
        })
        .collect();
    Some((
        (left, top, (right - left).max(1.0), (bottom - top).max(1.0)),
        moved,
    ))
}

impl WordDocument {
    /// Reads an ink drawing's InkML part into its strokes, fitted to its box.
    pub(super) fn resolve_ink_part(
        image: &mut RunImage,
        doc_part: &DocumentPart,
        package: &OpenXmlPackage,
        rel_map: &HashMap<&str, &Relationship>,
    ) {
        let Some(ink) = image.ink.as_mut() else {
            return;
        };
        if !ink.traces.is_empty() {
            return;
        }
        let Some(relationship) = ink
            .relationship_id
            .as_deref()
            .and_then(|id| rel_map.get(id))
        else {
            return;
        };
        let path = Self::resolve_target_path(&doc_part.path, &relationship.target);
        let Some(xml) = package
            .get(&path)
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
        else {
            return;
        };
        let traces = parse_inkml(xml);
        ink.traces = match (image.width_px, image.height_px) {
            (Some(width), Some(height)) => fit_traces_to_box(traces, width, height),
            _ => fit_traces_to_box(traces, 0.0, 0.0),
        };
    }

    /// The `mc:AlternateContent` Word writes for ink: the drawing over the
    /// InkML part, then `fallback` (the picture drawing) for consumers that
    /// do not read ink.
    pub(super) fn create_ink_alternate_content(
        doc: &mut DocxXmlDocument,
        relationship_id: &str,
        mut choice: XmlElement,
        fallback: Option<XmlElement>,
    ) -> XmlElement {
        // The choice's graphic is the content part.
        let content_part = Self::ink_content_part_element(doc, relationship_id, &choice);
        for child in choice.children.iter_mut() {
            let XmlNode::Element(container) = child else {
                continue;
            };
            if container.name != "wp:anchor" && container.name != "wp:inline" {
                continue;
            }
            if let Some(position) = container.children.iter().position(
                |node| matches!(node, XmlNode::Element(element) if element.name == "a:graphic"),
            ) {
                container.children[position] = XmlNode::Element(content_part.clone());
            }
        }
        let choice = doc.create_element(
            "mc:Choice",
            vec![XmlAttribute {
                name: "Requires".to_string(),
                value: "wpi".to_string(),
            }],
            vec![XmlNode::Element(choice)],
        );
        let mut children = vec![XmlNode::Element(choice)];
        if let Some(fallback) = fallback {
            children.push(XmlNode::Element(doc.create_element(
                "mc:Fallback",
                Vec::new(),
                vec![XmlNode::Element(fallback)],
            )));
        }
        doc.create_element("mc:AlternateContent", Vec::new(), children)
    }

    fn ink_content_part_element(
        doc: &mut DocxXmlDocument,
        relationship_id: &str,
        drawing: &XmlElement,
    ) -> XmlElement {
        // The extent the drawing already carries.
        let extent = drawing
            .children
            .iter()
            .filter_map(|node| match node {
                XmlNode::Element(container) => Some(container),
                _ => None,
            })
            .flat_map(|container| container.children.iter())
            .find_map(|node| match node {
                XmlNode::Element(element) if element.name == "wp:extent" => {
                    let value = |name: &str| {
                        element
                            .attributes
                            .iter()
                            .find(|attribute| attribute.name == name)
                            .map(|attribute| attribute.value.clone())
                            .unwrap_or_else(|| "0".to_string())
                    };
                    Some((value("cx"), value("cy")))
                }
                _ => None,
            })
            .unwrap_or_else(|| ("0".to_string(), "0".to_string()));
        let attribute = |name: &str, value: &str| XmlAttribute {
            name: name.to_string(),
            value: value.to_string(),
        };
        let off = doc.create_element(
            "a:off",
            vec![attribute("x", "0"), attribute("y", "0")],
            Vec::new(),
        );
        let ext = doc.create_element(
            "a:ext",
            vec![attribute("cx", &extent.0), attribute("cy", &extent.1)],
            Vec::new(),
        );
        let xfrm = doc.create_element(
            "w14:xfrm",
            Vec::new(),
            vec![XmlNode::Element(off), XmlNode::Element(ext)],
        );
        let c_nv = doc.create_element("w14:cNvContentPartPr", Vec::new(), Vec::new());
        let nv = doc.create_element(
            "w14:nvContentPartPr",
            Vec::new(),
            vec![XmlNode::Element(c_nv)],
        );
        doc.create_element(
            "w14:contentPart",
            vec![
                attribute("bwMode", "auto"),
                attribute("r:id", relationship_id),
            ],
            vec![XmlNode::Element(nv), XmlNode::Element(xfrm)],
        )
    }

    /// The ink drawings in body paragraphs, in document order.
    pub fn ink_drawings(&self) -> Vec<InkDrawingInfo> {
        let mut drawings = Vec::new();
        for paragraph_index in 0..self.paragraph_count() {
            if !self.is_body_paragraph_surface(paragraph_index) {
                continue;
            }
            let Some(paragraph) = self.paragraph(paragraph_index) else {
                continue;
            };
            for (run_index, run) in paragraph.runs.iter().enumerate() {
                let Some(image) = run.image.as_ref() else {
                    continue;
                };
                let Some(ink) = image.ink.as_ref() else {
                    continue;
                };
                let page = |position: Option<&ImagePosition>| {
                    position.is_some_and(|position| {
                        position.relative_from.as_deref() == Some("page")
                            && position.align.is_none()
                    })
                };
                drawings.push(InkDrawingInfo {
                    paragraph_index,
                    run_index,
                    page_relative: page(image.layout.position_h.as_ref())
                        && page(image.layout.position_v.as_ref()),
                    x_px: image
                        .layout
                        .position_h
                        .as_ref()
                        .and_then(|position| position.offset_px)
                        .unwrap_or(0.0),
                    y_px: image
                        .layout
                        .position_v
                        .as_ref()
                        .and_then(|position| position.offset_px)
                        .unwrap_or(0.0),
                    width_px: image.width_px.unwrap_or(1.0),
                    height_px: image.height_px.unwrap_or(1.0),
                    traces: ink.traces.clone(),
                });
            }
        }
        drawings
    }

    /// Replaces the body's ink drawings with `specs`: kept ones stay as they
    /// are, the rest go (their parts with them, once nothing else uses
    /// them), and new ones go in as Word ink with a picture fallback.
    pub fn set_ink_drawings(&mut self, specs: &[InkDrawingSpec]) -> Result<(), String> {
        let keep: HashSet<(usize, usize)> = specs
            .iter()
            .filter_map(|spec| match spec {
                InkDrawingSpec::Keep {
                    paragraph_index,
                    run_index,
                } => Some((*paragraph_index, *run_index)),
                InkDrawingSpec::New { .. } => None,
            })
            .collect();
        for spec in specs {
            if let InkDrawingSpec::New {
                paragraph_index,
                traces,
            } = spec
            {
                if !self.is_body_paragraph_surface(*paragraph_index) {
                    return Err(format!(
                        "ink paragraph {paragraph_index} is not a body paragraph"
                    ));
                }
                if traces.iter().all(|trace| trace.points.is_empty())
                    || traces
                        .iter()
                        .flat_map(|trace| &trace.points)
                        .any(|point| !point.0.is_finite() || !point.1.is_finite())
                {
                    return Err("an ink drawing needs strokes with finite points".to_string());
                }
            }
        }

        // Take out the ink drawings not kept.
        let mut removed = HashSet::new();
        for paragraph_index in 0..self.paragraph_count() {
            if !self.is_body_paragraph_surface(paragraph_index) {
                continue;
            }
            let Some(paragraph) = self.paragraph_mut(paragraph_index) else {
                continue;
            };
            let before = paragraph.runs.len();
            let mut run_index = 0;
            paragraph.runs.retain(|run| {
                let index = run_index;
                run_index += 1;
                let Some(image) = run.image.as_ref().filter(|image| image.ink.is_some()) else {
                    return true;
                };
                if keep.contains(&(paragraph_index, index)) {
                    return true;
                }
                removed.extend(image.relationship_id.clone());
                removed.extend(
                    image
                        .ink
                        .as_ref()
                        .and_then(|ink| ink.relationship_id.clone()),
                );
                false
            });
            if paragraph.runs.len() != before {
                self.sync_paragraph_text(paragraph_index);
            }
        }
        self.drop_unused_ink_relationships(removed)?;

        for spec in specs {
            let InkDrawingSpec::New {
                paragraph_index,
                traces,
            } = spec
            else {
                continue;
            };
            let Some(((x, y, width, height), local)) = traces_in_box(traces) else {
                continue;
            };
            let ink_relationship = self.add_ink_part(&write_inkml(&local))?;
            let png = rasterize_ink(&local, width, height);
            let picture = self.add_image_media_part(&png, super::InlineImageMime::Png, "ink")?;
            let data_uri = format!("data:image/png;base64,{}", BASE64.encode(&png));
            let run = Run {
                text: String::new(),
                properties: None,
                image: Some(RunImage {
                    relationship_id: Some(picture.id),
                    width_px: Some(width),
                    height_px: Some(height),
                    data_uri: Some(data_uri),
                    layout: RunImageLayout {
                        kind: RunImageKind::Floating,
                        position_h: Some(ImagePosition {
                            relative_from: Some("page".to_string()),
                            offset_px: Some(x),
                            align: None,
                        }),
                        position_v: Some(ImagePosition {
                            relative_from: Some("page".to_string()),
                            offset_px: Some(y),
                            align: None,
                        }),
                        wrap: WrapMode::None,
                        behind_doc: false,
                        ..RunImageLayout::default()
                    },
                    ink: Some(Box::new(InkContent {
                        relationship_id: Some(ink_relationship.id),
                        traces: local,
                    })),
                    ..RunImage::default()
                }),
                text_box: None,
                math: None,
                field: None,
                hyperlink: None,
                node_id: None,
                text_node_id: None,
                revision_node_id: None,
                comment_reference_ids: Vec::new(),
                footnote_reference_ids: Vec::new(),
                endnote_reference_ids: Vec::new(),
                note_separator: None,
                has_footnote_ref: false,
                has_endnote_ref: false,
                revision: None,
            };
            self.paragraph_mut(*paragraph_index)
                .ok_or_else(|| "the ink's paragraph disappeared".to_string())?
                .runs
                .push(run);
            self.sync_paragraph_text(*paragraph_index);
        }
        self.rebuild_paragraph_surfaces();
        Ok(())
    }

    /// Adds an InkML part related from the document.
    fn add_ink_part(&mut self, inkml: &str) -> Result<Relationship, String> {
        let path = (1..)
            .map(|index| format!("word/ink/ink{index}.xml"))
            .find(|candidate| {
                !self.parts.contains_key(candidate)
                    && !self
                        .package
                        .as_ref()
                        .is_some_and(|package| package.files.contains_key(candidate))
                    || self.removed_parts.contains(candidate)
            })
            .expect("an unused ink part name");
        let relationship = {
            let document_part = self
                .document_part
                .as_mut()
                .ok_or_else(|| "main document part is unavailable".to_string())?;
            let relationship = Relationship {
                id: Self::next_relationship_id(&document_part.relationships),
                r#type: INK_RELATIONSHIP_TYPE.to_string(),
                target: path.strip_prefix("word/").unwrap_or(&path).to_string(),
                target_mode: None,
            };
            document_part.relationships.push(relationship.clone());
            relationship
        };
        if let Some(document_part) = self.document_part.as_ref() {
            if let Some(part) = self.parts.get_mut(&document_part.path) {
                part.relationships = document_part.relationships.clone();
            }
        }
        self.ensure_document_relationships_part_has_relationship(&relationship)?;
        self.ensure_content_types_has_override(&path, INK_CONTENT_TYPE)?;
        self.removed_parts.remove(&path);
        self.upsert_xml_part_content(&path, inkml.to_string());
        Ok(relationship)
    }

    /// Drops relationships (and their parts) no run uses any more.
    fn drop_unused_ink_relationships(&mut self, mut ids: HashSet<String>) -> Result<(), String> {
        let still_used: HashSet<String> = (0..self.paragraph_count())
            .filter_map(|index| self.paragraph(index))
            .flat_map(|paragraph| paragraph.runs.iter())
            .filter_map(|run| run.image.as_ref())
            .flat_map(|image| {
                image.relationship_id.clone().into_iter().chain(
                    image
                        .ink
                        .as_ref()
                        .and_then(|ink| ink.relationship_id.clone()),
                )
            })
            .collect();
        ids.retain(|id| !still_used.contains(id));
        if ids.is_empty() {
            return Ok(());
        }
        let document_path = self
            .document_part
            .as_ref()
            .map(|part| part.path.clone())
            .unwrap_or_else(|| "word/document.xml".to_string());
        let paths: Vec<String> = self
            .document_part
            .as_ref()
            .into_iter()
            .flat_map(|part| part.relationships.iter())
            .filter(|relationship| ids.contains(&relationship.id))
            .map(|relationship| Self::resolve_target_path(&document_path, &relationship.target))
            .collect();
        if let Some(document_part) = self.document_part.as_mut() {
            document_part
                .relationships
                .retain(|relationship| !ids.contains(&relationship.id));
            if let Some(part) = self.parts.get_mut(&document_part.path) {
                part.relationships = document_part.relationships.clone();
            }
        }
        self.remove_document_relationships(&ids)?;
        for path in paths {
            self.binary_parts.remove(&path);
            self.parts.remove(&path);
            if self
                .package
                .as_ref()
                .is_some_and(|package| package.files.contains_key(&path))
            {
                self.removed_parts.insert(path);
            }
        }
        Ok(())
    }
}

/// A transparent PNG of `traces` (box px), the fallback picture Word keeps
/// beside the ink for consumers that do not read it.
pub fn rasterize_ink(traces: &[InkTrace], width: f64, height: f64) -> Vec<u8> {
    let (w, h) = (
        width.ceil().max(1.0) as usize,
        height.ceil().max(1.0) as usize,
    );
    let (w, h) = (w.min(4096), h.min(4096));
    let mut pixels = vec![0f32; w * h * 4];
    for trace in traces {
        let color = parse_color(&trace.brush.color);
        let opacity = match trace.brush.tool {
            InkTool::Highlighter => 0.5,
            InkTool::Pencil => 0.8,
            InkTool::Pen => 1.0,
        } * (1.0 - f32::from(trace.brush.transparency) / 255.0);
        let half = (trace.brush.width_px.max(1.0) / 2.0) as f32;
        let points: Vec<(f32, f32)> = trace
            .points
            .iter()
            .map(|&(x, y)| (x as f32, y as f32))
            .collect();
        let segments: Vec<((f32, f32), (f32, f32))> = if points.len() == 1 {
            vec![(points[0], points[0])]
        } else {
            points.windows(2).map(|pair| (pair[0], pair[1])).collect()
        };
        // Coverage per pixel for the whole stroke, so overlaps do not darken.
        let mut coverage = vec![0f32; w * h];
        for (a, b) in segments {
            let left = (a.0.min(b.0) - half - 1.0).floor().max(0.0) as usize;
            let top = (a.1.min(b.1) - half - 1.0).floor().max(0.0) as usize;
            let right = ((a.0.max(b.0) + half + 1.0).ceil() as usize).min(w);
            let bottom = ((a.1.max(b.1) + half + 1.0).ceil() as usize).min(h);
            for y in top..bottom {
                for x in left..right {
                    let p = (x as f32 + 0.5, y as f32 + 0.5);
                    let distance = segment_distance(p, a, b);
                    let value = (half - distance + 0.5).clamp(0.0, 1.0);
                    let slot = &mut coverage[y * w + x];
                    *slot = slot.max(value);
                }
            }
        }
        for (index, value) in coverage.into_iter().enumerate() {
            if value <= 0.0 {
                continue;
            }
            let alpha = value * opacity;
            let pixel = &mut pixels[index * 4..index * 4 + 4];
            let out_alpha = alpha + pixel[3] * (1.0 - alpha);
            for channel in 0..3 {
                let source = color[channel] * alpha;
                let destination = pixel[channel] * pixel[3] * (1.0 - alpha);
                pixel[channel] = if out_alpha > 0.0 {
                    (source + destination) / out_alpha
                } else {
                    0.0
                };
            }
            pixel[3] = out_alpha;
        }
    }
    let bytes: Vec<u8> = pixels
        .chunks(4)
        .flat_map(|pixel| {
            [
                (pixel[0] * 255.0).round() as u8,
                (pixel[1] * 255.0).round() as u8,
                (pixel[2] * 255.0).round() as u8,
                (pixel[3] * 255.0).round() as u8,
            ]
        })
        .collect();
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        if let Ok(mut writer) = encoder.write_header() {
            let _ = writer.write_image_data(&bytes);
        }
    }
    png
}

fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (x, y) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - x).powi(2) + (p.1 - y).powi(2)).sqrt()
}

fn parse_color(color: &str) -> [f32; 3] {
    let hex = color.trim_start_matches('#');
    let channel = |index: usize| {
        hex.get(index..index + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .unwrap_or(0) as f32
            / 255.0
    };
    [channel(0), channel(2), channel(4)]
}
