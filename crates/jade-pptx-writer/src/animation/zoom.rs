//! 挿入 > ズーム: Summary, Slide and Section Zoom frames, written as
//! PowerPoint writes them — a 2016 zoom `graphicFrame` in an
//! `mc:AlternateContent` choice, with a linked picture as the fallback
//! for readers that predate zoom.

use jade_opc::OpcPackage;
use jade_pptx_reader::animation::{XmlNode, ZoomKind};
use sha2::{Digest, Sha256};

use crate::xml::animation::part_relationship;
use crate::{EditSession, Result, WriteError};

const SLIDE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";
const P14: &str = "http://schemas.microsoft.com/office/powerpoint/2010/main";

/// A section of the presentation (`p14:section`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub id: String,
    pub slide_ids: Vec<u32>,
}

/// The presentation's sections, in order.
pub fn sections(bytes: &[u8]) -> Result<Vec<Section>> {
    let package = OpcPackage::open(bytes)?;
    let Some(xml) = package.part_str("ppt/presentation.xml")? else {
        return Ok(Vec::new());
    };
    let root = XmlNode::parse(&xml).map_err(|error| WriteError::Xml(error.to_string()))?;
    let mut out = Vec::new();
    for list in root.descendants("sectionLst") {
        for section in list.children_named("section") {
            out.push(Section {
                name: section.attr("name").unwrap_or_default().to_string(),
                id: section.attr("id").unwrap_or_default().to_string(),
                slide_ids: section
                    .descendants("sldId")
                    .into_iter()
                    .filter_map(|node| node.attr("id")?.parse().ok())
                    .collect(),
            });
        }
    }
    let _ = P14;
    Ok(out)
}

/// What one zoom (or one entry of a summary zoom) jumps to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZoomTarget {
    /// A slide, by its `p:sldId` id.
    Slide(u32),
    /// A section, by its id; it opens on the section's first slide.
    Section(String),
}

/// One zoom to insert: its image (a PNG of the slide it shows) and where
/// it sits, in EMU — for a summary zoom relative to the frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ZoomEntry {
    pub target: ZoomTarget,
    pub image: Vec<u8>,
    pub rect: (i64, i64, i64, i64),
}

/// ズームの書式設定: Return to Zoom, the transition's length, and whether
/// the zoom shows the target's background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoomOptions {
    pub return_to_parent: bool,
    pub transition_ms: u32,
    pub show_background: bool,
}

impl ZoomOptions {
    /// PowerPoint's defaults: a slide zoom stays on the slide it opens,
    /// a section or summary zoom comes back.
    pub fn defaults(kind: ZoomKind) -> Self {
        Self {
            return_to_parent: kind != ZoomKind::Slide,
            transition_ms: 1000,
            show_background: true,
        }
    }
}

fn uri(kind: ZoomKind) -> (&'static str, &'static str, &'static str, &'static str) {
    match kind {
        ZoomKind::Slide => (
            jade_pptx_reader::animation::zoom::NS_SLIDE_ZOOM,
            "pslz",
            "sldZm",
            "sldZmObj",
        ),
        ZoomKind::Section => (
            jade_pptx_reader::animation::zoom::NS_SECTION_ZOOM,
            "psez",
            "sectionZm",
            "sectionZmObj",
        ),
        ZoomKind::Summary => (
            jade_pptx_reader::animation::zoom::NS_SUMMARY_ZOOM,
            "pssz",
            "summaryZm",
            "summaryZmObj",
        ),
    }
}

fn guid(seed: &str) -> String {
    let digest = Sha256::digest(seed.as_bytes());
    let hex: String = digest.iter().take(16).map(|b| format!("{b:02X}")).collect();
    format!(
        "{{{}-{}-{}-{}-{}}}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn xfrm(rect: (i64, i64, i64, i64)) -> String {
    format!(
        r#"<a:xfrm><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></a:xfrm>"#,
        rect.0, rect.1, rect.2, rect.3
    )
}

fn next_shape_id(xml: &str) -> u32 {
    let mut max = 1;
    let mut rest = xml;
    while let Some(at) = rest.find("cNvPr id=\"") {
        rest = &rest[at + 10..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if let Ok(id) = digits.parse::<u32>() {
            max = max.max(id);
        }
    }
    max + 1
}

fn slide_part_of(bytes: &[u8], slide_id: u32) -> Result<String> {
    let presentation = jade_pptx_reader::read_pptx(bytes)?;
    presentation
        .slides
        .iter()
        .find(|slide| slide.id == Some(slide_id))
        .map(|slide| slide.part_name.clone())
        .ok_or(WriteError::NoShape(slide_id))
}

/// Inserts a zoom on slide `slide`: one frame at `frame` (EMU) holding
/// `entries` — one for a slide or section zoom, one per section for a
/// summary zoom. Returns the new frame's shape id.
pub fn insert_zoom(
    bytes: &[u8],
    slide: usize,
    kind: ZoomKind,
    frame: (i64, i64, i64, i64),
    entries: &[ZoomEntry],
    options: ZoomOptions,
) -> Result<(Vec<u8>, u32)> {
    if entries.is_empty() {
        return Err(WriteError::Xml("a zoom needs a target".into()));
    }
    let all_sections = sections(bytes)?;
    let (mut package, slide_part, xml) = crate::slide_xml(bytes, slide)?;
    let id = next_shape_id(&xml);
    let (namespace, prefix, container, object) = uri(kind);
    let label = match kind {
        ZoomKind::Slide => "スライド ズーム",
        ZoomKind::Section => "セクション ズーム",
        ZoomKind::Summary => "サマリー ズーム",
    };
    let name = format!("{label} {}", id - 1);
    let mut objects = String::new();
    let mut fallbacks = String::new();
    for (index, entry) in entries.iter().enumerate() {
        let image_rid = crate::xml::pictures::install_slide_image(
            &mut package,
            &slide_part,
            &xml,
            &entry.image,
        )?;
        let (target_attr, jump_slide) = match &entry.target {
            ZoomTarget::Slide(slide_id) => (format!(r#"sldId="{slide_id}""#), *slide_id),
            ZoomTarget::Section(section) => {
                let first = all_sections
                    .iter()
                    .find(|candidate| &candidate.id == section)
                    .and_then(|candidate| candidate.slide_ids.first().copied())
                    .ok_or_else(|| WriteError::Xml(format!("no section {section}")))?;
                (format!(r#"sectionId="{section}""#), first)
            }
        };
        let target_part = slide_part_of(bytes, jump_slide)?;
        let link_rid = part_relationship(
            &mut package,
            &slide_part,
            SLIDE_REL,
            &target_part,
            "rIdJadeZoom",
        )?;
        let local = if kind == ZoomKind::Summary {
            entry.rect
        } else {
            (0, 0, frame.2, frame.3)
        };
        let absolute = if kind == ZoomKind::Summary {
            (
                frame.0 + entry.rect.0,
                frame.1 + entry.rect.1,
                entry.rect.2,
                entry.rect.3,
            )
        } else {
            frame
        };
        let zoom_id = guid(&format!("{slide_part}/{id}/{index}/{target_attr}"));
        let background = if options.show_background {
            String::new()
        } else {
            r#" showBg="0""#.to_string()
        };
        objects.push_str(&format!(
            r#"<{prefix}:{object} {target_attr} cId="0"><{prefix}:zmPr id="{zoom_id}" returnToParent="{ret}" transitionDur="{dur}"{background}><p166:blipFill xmlns:p166="http://schemas.microsoft.com/office/powerpoint/2016/6/main"><a:blip r:embed="{image_rid}"/><a:stretch><a:fillRect/></a:stretch></p166:blipFill><p166:spPr xmlns:p166="http://schemas.microsoft.com/office/powerpoint/2016/6/main">{xfrm}<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:ln w="3175"><a:solidFill><a:prstClr val="ltGray"/></a:solidFill></a:ln></p166:spPr></{prefix}:zmPr></{prefix}:{object}>"#,
            ret = u8::from(options.return_to_parent),
            dur = options.transition_ms,
            xfrm = xfrm(local),
        ));
        let pic_id = if kind == ZoomKind::Summary {
            id + 1 + index as u32
        } else {
            id
        };
        fallbacks.push_str(&format!(
            r#"<p:pic><p:nvPicPr><p:cNvPr id="{pic_id}" name="{name}"><a:hlinkClick r:id="{link_rid}" action="ppaction://hlinksldjump"/></p:cNvPr><p:cNvPicPr><a:picLocks noGrp="1" noRot="1" noChangeAspect="1"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="{image_rid}"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{xfrm}<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:ln w="3175"><a:solidFill><a:prstClr val="ltGray"/></a:solidFill></a:ln></p:spPr></p:pic>"#,
            xfrm = xfrm(absolute),
        ));
    }
    let fallback = if kind == ZoomKind::Summary {
        format!(
            r#"<p:grpSp><p:nvGrpSpPr><p:cNvPr id="{id}" name="{name}"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/><a:chOff x="{x}" y="{y}"/><a:chExt cx="{w}" cy="{h}"/></a:xfrm></p:grpSpPr>{fallbacks}</p:grpSp>"#,
            x = frame.0,
            y = frame.1,
            w = frame.2,
            h = frame.3
        )
    } else {
        fallbacks
    };
    let markup = format!(
        r#"<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice xmlns:{prefix}="{namespace}" Requires="{prefix}"><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="{id}" name="{name}"/><p:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect="1"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></p:xfrm><a:graphic><a:graphicData uri="{namespace}"><{prefix}:{container}>{objects}</{prefix}:{container}></a:graphicData></a:graphic></p:graphicFrame></mc:Choice><mc:Fallback>{fallback}</mc:Fallback></mc:AlternateContent>"#,
        x = frame.0,
        y = frame.1,
        w = frame.2,
        h = frame.3
    );
    let close = xml
        .rfind("</p:spTree>")
        .ok_or_else(|| WriteError::Xml("the slide has no shape tree".into()))?;
    let mut changed = xml.clone();
    changed.insert_str(close, &markup);
    package.replace(&slide_part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok((result, id))
}

/// Rewrites the zoom options of frame `shape_id` on slide `slide`.
pub fn set_zoom_options(
    bytes: &[u8],
    slide: usize,
    shape_id: u32,
    options: ZoomOptions,
) -> Result<Vec<u8>> {
    let (mut package, slide_part, xml) = crate::slide_xml(bytes, slide)?;
    let marker = format!(r#"cNvPr id="{shape_id}""#);
    let Some(at) = xml.find(&marker) else {
        return Err(WriteError::NoShape(shape_id));
    };
    let frame_start = xml[..at]
        .rfind("<p:graphicFrame")
        .ok_or(WriteError::NoShape(shape_id))?;
    let frame_end = xml[at..]
        .find("</p:graphicFrame>")
        .map(|end| at + end)
        .ok_or(WriteError::NoShape(shape_id))?;
    let frame = &xml[frame_start..frame_end];
    let mut rewritten = String::with_capacity(frame.len());
    let mut rest = frame;
    while let Some(open) = rest.find(":zmPr ") {
        let tag_end = rest[open..]
            .find('>')
            .map(|end| open + end)
            .unwrap_or(rest.len());
        rewritten.push_str(&rest[..open]);
        let tag = &rest[open..tag_end];
        let mut node_attrs: Vec<String> = Vec::new();
        for part in tag
            .trim_start_matches(":zmPr")
            .split('"')
            .collect::<Vec<_>>()
            .chunks(2)
        {
            if let [key, value] = part {
                let key = key.trim().trim_end_matches('=').trim();
                if !matches!(key, "returnToParent" | "transitionDur" | "showBg") && !key.is_empty()
                {
                    node_attrs.push(format!(r#"{key}="{value}""#));
                }
            }
        }
        node_attrs.push(format!(
            r#"returnToParent="{}""#,
            u8::from(options.return_to_parent)
        ));
        node_attrs.push(format!(r#"transitionDur="{}""#, options.transition_ms));
        if !options.show_background {
            node_attrs.push(r#"showBg="0""#.to_string());
        }
        let closing = if tag.ends_with('/') { "/" } else { "" };
        rewritten.push_str(&format!(":zmPr {}{closing}", node_attrs.join(" ")));
        rest = &rest[tag_end..];
    }
    rewritten.push_str(rest);
    let changed = format!("{}{}{}", &xml[..frame_start], rewritten, &xml[frame_end..]);
    if changed == xml {
        return Ok(bytes.to_vec());
    }
    package.replace(&slide_part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok(result)
}

impl EditSession {
    /// 挿入 > ズーム as one step of Undo; returns the new frame's id.
    pub fn insert_zoom(
        &mut self,
        slide: usize,
        kind: ZoomKind,
        frame: (i64, i64, i64, i64),
        entries: &[ZoomEntry],
        options: ZoomOptions,
    ) -> Result<Option<u32>> {
        let (next, id) = insert_zoom(self.bytes(), slide, kind, frame, entries, options)?;
        Ok(self.commit(next).then_some(id))
    }

    /// ズーム > ズームのオプション as one step of Undo.
    pub fn set_zoom_options(
        &mut self,
        slide: usize,
        shape_id: u32,
        options: ZoomOptions,
    ) -> Result<bool> {
        let next = set_zoom_options(self.bytes(), slide, shape_id, options)?;
        Ok(self.commit(next))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_pptx_reader::animation::read_slide_animation;

    fn deck() -> Vec<u8> {
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../public/fixtures/sample-deck.pptx"
        ))
        .unwrap()
    }

    fn png() -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(4, 3, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn inserts_a_slide_zoom_that_reads_back_and_takes_options() {
        let bytes = deck();
        let presentation = jade_pptx_reader::read_pptx(&bytes).unwrap();
        let target = presentation.slides.last().unwrap().id.unwrap();
        let entry = ZoomEntry {
            target: ZoomTarget::Slide(target),
            image: png(),
            rect: (0, 0, 0, 0),
        };
        let frame = (914_400, 914_400, 3_048_000, 1_714_500);
        let (zoomed, id) = insert_zoom(
            &bytes,
            0,
            ZoomKind::Slide,
            frame,
            &[entry],
            ZoomOptions::defaults(ZoomKind::Slide),
        )
        .unwrap();
        let (_, _, xml) = crate::slide_xml(&zoomed, 0).unwrap();
        assert!(xml.contains(r#"<mc:Choice xmlns:pslz="http://schemas.microsoft.com/office/powerpoint/2016/slidezoom" Requires="pslz">"#));
        assert!(xml.contains(r#"action="ppaction://hlinksldjump""#));
        let zooms = read_slide_animation(&xml).unwrap().zooms;
        assert_eq!(zooms.len(), 1);
        assert_eq!(zooms[0].shape_id, id);
        assert_eq!(zooms[0].slide_id, Some(target));
        assert!(!zooms[0].return_to_parent);
        assert_eq!(zooms[0].rect, frame);

        let options = ZoomOptions {
            return_to_parent: true,
            transition_ms: 1500,
            show_background: false,
        };
        let changed = set_zoom_options(&zoomed, 0, id, options).unwrap();
        let (_, _, xml) = crate::slide_xml(&changed, 0).unwrap();
        let zoom = &read_slide_animation(&xml).unwrap().zooms[0];
        assert!(zoom.return_to_parent && zoom.hide_background);
        assert_eq!(zoom.transition_ms, 1500);
    }
}
