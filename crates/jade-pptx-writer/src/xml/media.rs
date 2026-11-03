//! Ink, video and audio in a slide's XML: the content parts ink sits in,
//! media pictures with their `p14:media` extension, and the `p:timing`
//! nodes that make PowerPoint play them.

use super::slides::{append_child, content_types, p_ns, r_ns};
use super::{apply_patches, child, drawing_ns, edit_tag, parse_document_nodes, parse_nodes, Node};
use crate::{Result, WriteError};
use jade_opc::{rels_part_name, resolve_target, OpcPackage};
use jade_pptx_reader::{MediaBookmark, MediaKind, MediaPlayback, MediaStart, Transform};
use std::collections::{BTreeMap, BTreeSet, HashSet};

const P14: &str = jade_pptx_reader::media::P14_NAMESPACE;
const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const RELS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const MEDIA_EXT_URI: &str = jade_pptx_reader::media::MEDIA_EXT_URI;

fn escape(value: &str) -> String {
    quick_xml::escape::escape(value).into_owned()
}

/// The `xmlns` declarations a fragment carries, so it reads the same
/// whatever prefixes the slide uses.
fn namespaces(strict: bool) -> String {
    format!(
        r#" xmlns:p="{}" xmlns:a="{}" xmlns:r="{}""#,
        p_ns(strict),
        drawing_ns(strict),
        r_ns(strict)
    )
}

fn xfrm(transform: Transform, prefix: &str) -> String {
    let (x, y) = transform.offset_emu;
    let (cx, cy) = transform.extent_emu;
    format!(
        r#"<{prefix}:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{cx}" cy="{cy}"/></{prefix}:xfrm>"#
    )
}

/// Every shape id the slide uses: `cNvPr` in any namespace (ink's is
/// `p14:cNvPr`) and every reference to one.
fn used_ids(nodes: &[Node]) -> HashSet<u32> {
    let mut used = HashSet::new();
    for node in nodes {
        if node.name == "cNvPr" {
            if let Some(id) = node.attr("id").and_then(|v| v.parse().ok()) {
                used.insert(id);
            }
        }
        for (key, value) in &node.attrs {
            if matches!(key.rsplit(':').next(), Some("spid" | "spId" | "shapeId")) {
                if let Ok(id) = value.parse() {
                    used.insert(id);
                }
            }
        }
    }
    used
}

fn fresh_id(used: &HashSet<u32>) -> Result<u32> {
    used.iter()
        .max()
        .copied()
        .unwrap_or(1)
        .checked_add(1)
        .or_else(|| (2..=u32::MAX).find(|id| !used.contains(id)))
        .ok_or_else(|| WriteError::Xml("no unused shape id".into()))
}

fn fresh_name(nodes: &[Node], stem: &str) -> String {
    let names: HashSet<_> = nodes
        .iter()
        .filter(|n| n.name == "cNvPr")
        .filter_map(|n| n.attr("name"))
        .collect();
    (1..)
        .map(|n| format!("{stem} {n}"))
        .find(|name| !names.contains(name.as_str()))
        .unwrap()
}

fn tree(nodes: &[Node]) -> Result<&Node> {
    child(nodes, &nodes[0], 1, "cSld")
        .and_then(|cs| child(nodes, cs, 1, "spTree"))
        .ok_or_else(|| WriteError::Xml("slide has no shape tree".into()))
}

/// Where a new object goes: last in the shape tree, before its extensions.
fn tree_end(nodes: &[Node]) -> Result<usize> {
    let tree = tree(nodes)?;
    if tree.empty {
        return Err(WriteError::Xml("empty shape tree".into()));
    }
    Ok(child(nodes, tree, 1, "extLst").map_or(tree.range.end, |node| node.outer.start))
}

/// Adds parts and relationships to a slide, a batch at a time.
struct SlideParts<'p> {
    package: &'p mut OpcPackage,
    slide_part: String,
    rels: String,
    used_rel_ids: HashSet<String>,
    strict: bool,
}

impl<'p> SlideParts<'p> {
    fn new(package: &'p mut OpcPackage, slide_part: &str, nodes: &[Node]) -> Result<Self> {
        let rels = package
            .part_str(&rels_part_name(slide_part))?
            .unwrap_or_else(|| format!("<Relationships xmlns=\"{RELS}\"/>"));
        let mut used_rel_ids: HashSet<String> = parse_document_nodes(&rels)?
            .iter()
            .filter_map(|node| node.attr("Id").map(str::to_owned))
            .collect();
        for node in nodes {
            for (key, value) in &node.attrs {
                if matches!(key.rsplit(':').next(), Some("id" | "embed" | "link")) {
                    used_rel_ids.insert(value.clone());
                }
            }
        }
        Ok(Self {
            package,
            slide_part: slide_part.to_string(),
            rels,
            used_rel_ids,
            strict: nodes[0].strict,
        })
    }

    pub(crate) fn package(&mut self) -> &mut OpcPackage {
        self.package
    }

    /// The part a relationship of this slide targets.
    pub(crate) fn target(&self, rel_id: &str) -> Option<(String, bool)> {
        let nodes = parse_document_nodes(&self.rels).ok()?;
        let node = nodes
            .iter()
            .find(|n| n.is(4, "Relationship") && n.attr("Id") == Some(rel_id))?;
        let external = node.attr("TargetMode") == Some("External");
        let target = node.attr("Target")?;
        Some(if external {
            (target.to_string(), true)
        } else {
            (resolve_target(&self.slide_part, target), false)
        })
    }

    /// A new relationship from the slide: its id.
    pub(crate) fn relate(&mut self, kind: &str, target: &str, external: bool) -> Result<String> {
        let rid = (1..)
            .map(|n| format!("rIdJadeMedia{n}"))
            .find(|rid| !self.used_rel_ids.contains(rid))
            .unwrap();
        self.used_rel_ids.insert(rid.clone());
        let (target, mode) = if external {
            (escape(target), " TargetMode=\"External\"")
        } else {
            (format!("/{}", escape(target)), "")
        };
        let relation = format!(
            "<jadePkg:Relationship xmlns:jadePkg=\"{RELS}\" Id=\"{rid}\" Type=\"{kind}\" Target=\"{target}\"{mode}/>"
        );
        let nodes = parse_document_nodes(&self.rels)?;
        let root = nodes
            .first()
            .filter(|node| node.is(4, "Relationships"))
            .ok_or_else(|| WriteError::Xml("missing relationships root".into()))?;
        self.rels = append_child(&self.rels, root, &relation);
        Ok(rid)
    }

    /// The `officeDocument` relationship type `suffix` (`/image`, `/video`…).
    pub(crate) fn office_type(&self, suffix: &str) -> String {
        format!("{}{suffix}", r_ns(self.strict))
    }

    /// A new part `dir/stem<N>.ext`, with its content type.
    pub(crate) fn add_part(
        &mut self,
        dir: &str,
        stem: &str,
        extension: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<String> {
        let taken: HashSet<String> = self
            .package
            .part_names()
            .iter()
            .map(|name| name.to_ascii_lowercase())
            .collect();
        let part = (1..)
            .map(|n| format!("{dir}/{stem}{n}.{extension}"))
            .find(|part| !taken.contains(&part.to_ascii_lowercase()))
            .unwrap();
        self.package.replace(&part, bytes);
        content_types(
            self.package,
            &BTreeMap::new(),
            &BTreeSet::new(),
            Some((&part, content_type)),
        )?;
        Ok(part)
    }

    /// A picture part and the slide's relationship to it.
    pub(crate) fn add_image(&mut self, image: &[u8]) -> Result<String> {
        let info = crate::picture_info(image)?;
        let extension = if info.mime_type == "image/png" {
            "png"
        } else {
            "jpeg"
        };
        let part = self.add_part(
            "ppt/media",
            "image",
            extension,
            info.mime_type,
            image.to_vec(),
        )?;
        let kind = self.office_type("/image");
        self.relate(&kind, &part, false)
    }

    /// Drops relationship `rel_id`; an internal part nothing else on the
    /// slide relates to goes with it.
    pub(crate) fn unrelate(&mut self, rel_id: &str) -> Result<()> {
        let nodes = parse_document_nodes(&self.rels)?;
        let Some(node) = nodes
            .iter()
            .find(|n| n.is(4, "Relationship") && n.attr("Id") == Some(rel_id))
        else {
            return Ok(());
        };
        let target = self.target(rel_id);
        self.rels = apply_patches(&self.rels, vec![(node.outer.clone(), String::new())]);
        if let Some((part, false)) = target {
            let shared = parse_document_nodes(&self.rels)?
                .iter()
                .filter(|n| n.is(4, "Relationship") && n.attr("TargetMode") != Some("External"))
                .filter_map(|n| n.attr("Target"))
                .any(|t| resolve_target(&self.slide_part, t) == part);
            if !shared {
                self.package.remove(&part);
            }
        }
        Ok(())
    }

    pub(crate) fn finish(self) {
        let name = rels_part_name(&self.slide_part);
        self.package.replace(&name, self.rels.into_bytes());
    }
}

// ---- ink -------------------------------------------------------------------

/// What to do to a slide's ink: strokes in slide px.
#[derive(Debug, Clone, PartialEq)]
pub enum InkEdit {
    /// A new ink drawing of these strokes.
    New(Vec<jade_pptx_reader::InkTrace>),
    /// The drawing `id`'s strokes become these (none deletes it).
    Replace {
        id: u32,
        traces: Vec<jade_pptx_reader::InkTrace>,
    },
    Delete(u32),
}

/// An ink drawing in the slide's XML: the element it is (the
/// `mc:AlternateContent`, or a bare `p:contentPart`), its name, and the
/// relationships of its InkML part and of its fallback picture.
struct InkNode<'a> {
    owner: &'a Node,
    name: String,
    ink_rel: Option<String>,
    image_rel: Option<String>,
}

fn rel_value(node: &Node, local: &str) -> Option<String> {
    node.attrs
        .iter()
        .find(|(key, _)| key.contains(':') && key.rsplit(':').next() == Some(local))
        .map(|(_, value)| value.clone())
}

fn find_ink<'a>(nodes: &'a [Node], id: u32) -> Option<InkNode<'a>> {
    for part in nodes.iter().filter(|n| n.is(1, "contentPart")) {
        let identity = super::objects::descendants(nodes, part)
            .iter()
            .find(|n| n.name == "cNvPr")?;
        if identity.attr("id").and_then(|v| v.parse().ok()) != Some(id) {
            continue;
        }
        let parent = part.parent.map(|i| &nodes[i]);
        let owner = match parent {
            Some(choice) if choice.is(3, "Choice") => &nodes[choice.parent?],
            _ => part,
        };
        let image_rel = (owner.is(3, "AlternateContent"))
            .then(|| child(nodes, owner, 3, "Fallback"))
            .flatten()
            .and_then(|fallback| {
                super::objects::descendants(nodes, fallback)
                    .iter()
                    .find(|n| n.is(2, "blip"))
            })
            .and_then(|blip| rel_value(blip, "embed"));
        return Some(InkNode {
            owner,
            name: identity.attr("name").unwrap_or_default().to_string(),
            ink_rel: rel_value(part, "id"),
            image_rel,
        });
    }
    None
}

fn ink_fragment(
    strict: bool,
    id: u32,
    name: &str,
    transform: Transform,
    ink_rel: &str,
    image_rel: &str,
) -> String {
    let ns = namespaces(strict);
    let name = escape(name);
    format!(
        r#"<mc:AlternateContent xmlns:mc="{MC}"{ns}><mc:Choice xmlns:p14="{P14}" Requires="p14"><p:contentPart p14:bwMode="auto" r:id="{ink_rel}"><p14:nvContentPartPr><p14:cNvPr id="{id}" name="{name}"/><p14:cNvContentPartPr/><p14:nvPr/></p14:nvContentPartPr>{}</p:contentPart></mc:Choice><mc:Fallback><p:pic><p:nvPicPr><p:cNvPr id="{id}" name="{name}"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="{image_rel}"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic></mc:Fallback></mc:AlternateContent>"#,
        xfrm(transform, "p14"),
        xfrm(transform, "a"),
    )
}

/// After ink drawing `id`'s fallback picture was moved or resized: its
/// `p14:xfrm`, the box its strokes fill, follows, so the strokes move and
/// stretch with it as in PowerPoint. Anything else comes back unchanged.
pub(crate) fn follow_ink_transform(xml: &str, id: u32, transform: Transform) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let Some(ink) = find_ink(&nodes, id) else {
        return Ok(xml.to_string());
    };
    let Some(box_node) = super::objects::descendants(&nodes, ink.owner)
        .iter()
        .find(|n| n.is(6, "xfrm"))
    else {
        return Ok(xml.to_string());
    };
    let mut out = xml.to_string();
    out.replace_range(box_node.outer.clone(), &xfrm(transform, "p14"));
    Ok(out)
}

/// The box (EMU) and the strokes moved into it, for strokes in slide px.
fn ink_box(
    traces: &[jade_pptx_reader::InkTrace],
) -> Option<(Transform, Vec<jade_pptx_reader::InkTrace>, (f64, f64))> {
    let ((x, y, width, height), local) = jade_ink::inkml::traces_in_box(traces)?;
    let emu = |px: f64| (px * 9525.0).round() as i64;
    Some((
        Transform {
            offset_emu: (emu(x), emu(y)),
            extent_emu: (emu(width).max(1), emu(height).max(1)),
            rotation_60000ths: 0,
            flip_h: false,
            flip_v: false,
        },
        local,
        (width, height),
    ))
}

/// Applies `edits` to the slide's ink: its XML, the InkML parts and the
/// fallback pictures. The ids of the new drawings come back.
pub(crate) fn edit_ink(
    package: &mut OpcPackage,
    slide_part: &str,
    xml: &str,
    edits: &[InkEdit],
) -> Result<(String, Vec<u32>)> {
    let nodes = parse_nodes(xml)?;
    let strict = nodes[0].strict;
    let mut used = used_ids(&nodes);
    let mut parts = SlideParts::new(package, slide_part, &nodes)?;
    let mut patches = Vec::new();
    let mut appended = String::new();
    let mut created = Vec::new();
    let mut names_taken: Vec<String> = Vec::new();
    for edit in edits {
        match edit {
            InkEdit::Delete(id) => {
                let ink = find_ink(&nodes, *id).ok_or(WriteError::NoShape(*id))?;
                patches.push((ink.owner.outer.clone(), String::new()));
            }
            InkEdit::Replace { id, traces } => {
                let ink = find_ink(&nodes, *id).ok_or(WriteError::NoShape(*id))?;
                let Some((transform, local, (width, height))) = ink_box(traces) else {
                    patches.push((ink.owner.outer.clone(), String::new()));
                    continue;
                };
                let inkml = jade_ink::inkml::write_inkml(&local).into_bytes();
                let ink_rel = match ink
                    .ink_rel
                    .as_deref()
                    .and_then(|rel| parts.target(rel).map(|target| (rel, target)))
                {
                    Some((rel, (target, false))) => {
                        parts.package().replace(&target, inkml);
                        rel.to_string()
                    }
                    _ => {
                        let part = parts.add_part(
                            "ppt/ink",
                            "ink",
                            "xml",
                            "application/inkml+xml",
                            inkml,
                        )?;
                        parts.relate(jade_ink::inkml::INK_RELATIONSHIP_TYPE, &part, false)?
                    }
                };
                let png = jade_ink::raster::rasterize_ink(&local, width, height);
                let image_rel = match ink
                    .image_rel
                    .as_deref()
                    .and_then(|rel| parts.target(rel).map(|target| (rel, target)))
                {
                    Some((rel, (target, false))) if target.ends_with(".png") => {
                        parts.package().replace(&target, png);
                        rel.to_string()
                    }
                    _ => parts.add_image(&png)?,
                };
                patches.push((
                    ink.owner.outer.clone(),
                    ink_fragment(strict, *id, &ink.name, transform, &ink_rel, &image_rel),
                ));
            }
            InkEdit::New(traces) => {
                let Some((transform, local, (width, height))) = ink_box(traces) else {
                    continue;
                };
                let id = fresh_id(&used)?;
                used.insert(id);
                let mut name = fresh_name(&nodes, "Ink");
                while names_taken.contains(&name) {
                    name = format!("{name}'");
                }
                names_taken.push(name.clone());
                let inkml = jade_ink::inkml::write_inkml(&local).into_bytes();
                let part =
                    parts.add_part("ppt/ink", "ink", "xml", "application/inkml+xml", inkml)?;
                let ink_rel = parts.relate(jade_ink::inkml::INK_RELATIONSHIP_TYPE, &part, false)?;
                let png = jade_ink::raster::rasterize_ink(&local, width, height);
                let image_rel = parts.add_image(&png)?;
                appended.push_str(&ink_fragment(
                    strict, id, &name, transform, &ink_rel, &image_rel,
                ));
                created.push(id);
            }
        }
    }
    parts.finish();
    if !appended.is_empty() {
        let at = tree_end(&nodes)?;
        patches.push((at..at, appended));
    }
    Ok((apply_patches(xml, patches), created))
}

/// インクを図形に変換's line or arrow: a straight connector from `from` to
/// `to` (slide px), in `color` (`#RRGGBB`) `width_pt` wide.
pub(crate) fn insert_line(
    xml: &str,
    from: (f64, f64),
    to: (f64, f64),
    color: &str,
    width_pt: f64,
    arrow: bool,
) -> Result<(String, u32)> {
    let nodes = parse_nodes(xml)?;
    let strict = nodes[0].strict;
    let id = fresh_id(&used_ids(&nodes))?;
    let name = fresh_name(
        &nodes,
        if arrow {
            "Straight Arrow Connector"
        } else {
            "Straight Connector"
        },
    );
    let emu = |px: f64| (px * 9525.0).round() as i64;
    let (l, t) = (from.0.min(to.0), from.1.min(to.1));
    let (w, h) = ((from.0 - to.0).abs(), (from.1 - to.1).abs());
    let flip_h = to.0 < from.0;
    let flip_v = to.1 < from.1;
    let mut flips = String::new();
    if flip_h {
        flips.push_str(r#" flipH="1""#);
    }
    if flip_v {
        flips.push_str(r#" flipV="1""#);
    }
    let color = color.trim_start_matches('#');
    if color.len() != 6 || !color.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(WriteError::InvalidShapeStyle);
    }
    let width = (width_pt.clamp(0.0, 1584.0) * 12700.0).round() as i64;
    let head = if arrow {
        r#"<a:tailEnd type="triangle"/>"#
    } else {
        ""
    };
    let ns = namespaces(strict);
    let fragment = format!(
        r#"<p:cxnSp{ns}><p:nvCxnSpPr><p:cNvPr id="{id}" name="{}"/><p:cNvCxnSpPr/><p:nvPr/></p:nvCxnSpPr><p:spPr><a:xfrm{flips}><a:off x="{}" y="{}"/><a:ext cx="{}" cy="{}"/></a:xfrm><a:prstGeom prst="{}"><a:avLst/></a:prstGeom><a:ln w="{width}"><a:solidFill><a:srgbClr val="{}"/></a:solidFill>{head}</a:ln></p:spPr></p:cxnSp>"#,
        escape(&name),
        emu(l),
        emu(t),
        emu(w).max(1),
        emu(h).max(1),
        if arrow { "straightConnector1" } else { "line" },
        color.to_ascii_uppercase(),
    );
    let at = tree_end(&nodes)?;
    Ok((apply_patches(xml, vec![(at..at, fragment)]), id))
}

// ---- media pictures ----------------------------------------------------------

/// A video or audio's media, as inserted.
#[derive(Debug, Clone, PartialEq)]
pub enum MediaSource<'a> {
    /// A file put in the presentation (デバイスから埋め込み).
    Embedded {
        bytes: &'a [u8],
        /// Its extension, `mp4`, `mov`, `m4a`, `mp3`, `wav`…
        extension: &'a str,
    },
    /// A file the presentation points at (ファイルにリンク): a `file:` URL or a
    /// path relative to the presentation.
    Linked { target: &'a str },
}

/// The content type PowerPoint gives media with this extension.
pub fn media_content_type(extension: &str) -> &'static str {
    match extension.to_ascii_lowercase().as_str() {
        "mp4" => "video/mp4",
        "m4v" => "video/x-m4v",
        "mov" | "qt" => "video/quicktime",
        "wmv" => "video/x-ms-wmv",
        "avi" => "video/x-msvideo",
        "mpg" | "mpeg" => "video/mpeg",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "wma" => "audio/x-ms-wma",
        "aac" => "audio/aac",
        "aif" | "aiff" => "audio/aiff",
        "mid" | "midi" => "audio/midi",
        _ => "application/octet-stream",
    }
}

fn media_relationships(
    parts: &mut SlideParts<'_>,
    kind: MediaKind,
    source: &MediaSource<'_>,
) -> Result<(String, String, bool)> {
    let file_type = parts.office_type(match kind {
        MediaKind::Video => "/video",
        MediaKind::Audio => "/audio",
    });
    let media_type = jade_pptx_reader::media::MEDIA_RELATIONSHIP_TYPE;
    Ok(match source {
        MediaSource::Embedded { bytes, extension } => {
            let extension = extension.trim_start_matches('.').to_ascii_lowercase();
            let part = parts.add_part(
                "ppt/media",
                "media",
                &extension,
                media_content_type(&extension),
                bytes.to_vec(),
            )?;
            let media = parts.relate(media_type, &part, false)?;
            let file = parts.relate(&file_type, &part, false)?;
            (media, file, false)
        }
        MediaSource::Linked { target } => {
            let media = parts.relate(media_type, target, true)?;
            let file = parts.relate(&file_type, target, true)?;
            (media, file, true)
        }
    })
}

fn p14_media_children(
    trim: (Option<f64>, Option<f64>),
    fade: (Option<f64>, Option<f64>),
    bookmarks: &[MediaBookmark],
) -> String {
    let ms = |value: f64| {
        let rounded = (value * 1000.0).round() / 1000.0;
        if rounded.fract() == 0.0 {
            format!("{}", rounded as i64)
        } else {
            format!("{rounded}")
        }
    };
    let mut out = String::new();
    if trim.0.is_some() || trim.1.is_some() {
        out.push_str("<p14:trim");
        if let Some(start) = trim.0 {
            out.push_str(&format!(" st=\"{}\"", ms(start)));
        }
        if let Some(end) = trim.1 {
            out.push_str(&format!(" end=\"{}\"", ms(end)));
        }
        out.push_str("/>");
    }
    if fade.0.is_some() || fade.1.is_some() {
        out.push_str("<p14:fade");
        if let Some(fade_in) = fade.0 {
            out.push_str(&format!(" in=\"{}\"", ms(fade_in)));
        }
        if let Some(fade_out) = fade.1 {
            out.push_str(&format!(" out=\"{}\"", ms(fade_out)));
        }
        out.push_str("/>");
    }
    if !bookmarks.is_empty() {
        out.push_str("<p14:bmkLst>");
        for bookmark in bookmarks {
            out.push_str(&format!(
                "<p14:bmk name=\"{}\" time=\"{}\"/>",
                escape(&bookmark.name),
                ms(bookmark.time_ms)
            ));
        }
        out.push_str("</p14:bmkLst>");
    }
    out
}

/// Inserts a video or audio picture, its poster `poster` over `transform`,
/// and the timing PowerPoint gives new media. Its id comes back.
#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_media(
    package: &mut OpcPackage,
    slide_part: &str,
    xml: &str,
    kind: MediaKind,
    source: &MediaSource<'_>,
    poster: &[u8],
    transform: Transform,
    duration_ms: Option<f64>,
) -> Result<(String, u32)> {
    let nodes = parse_nodes(xml)?;
    let strict = nodes[0].strict;
    let id = fresh_id(&used_ids(&nodes))?;
    let mut name = match kind {
        MediaKind::Video => fresh_name(&nodes, "Video"),
        MediaKind::Audio => fresh_name(&nodes, "Audio"),
    };
    if let MediaSource::Linked { target } = source {
        if let Some(file) = target.rsplit(['/', '\\']).next().filter(|f| !f.is_empty()) {
            name = percent_decode(file);
        }
    }
    let mut parts = SlideParts::new(package, slide_part, &nodes)?;
    let (media_rel, file_rel, linked) = media_relationships(&mut parts, kind, source)?;
    let poster_rel = parts.add_image(poster)?;
    parts.finish();
    let file_element = match kind {
        MediaKind::Video => "videoFile",
        MediaKind::Audio => "audioFile",
    };
    let media_attr = if linked { "r:link" } else { "r:embed" };
    let ns = namespaces(strict);
    let fragment = format!(
        r#"<p:pic{ns}><p:nvPicPr><p:cNvPr id="{id}" name="{}"><a:hlinkClick r:id="" action="ppaction://media"/></p:cNvPr><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr><p:nvPr><a:{file_element} r:link="{file_rel}"/><p:extLst><p:ext uri="{MEDIA_EXT_URI}"><p14:media xmlns:p14="{P14}" {media_attr}="{media_rel}"/></p:ext></p:extLst></p:nvPr></p:nvPicPr><p:blipFill><a:blip r:embed="{poster_rel}"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#,
        escape(&name),
        xfrm(transform, "a"),
    );
    let at = tree_end(&nodes)?;
    let inserted = apply_patches(xml, vec![(at..at, fragment)]);
    let timed = set_playback(&inserted, &MediaPlayback::new(id, kind), duration_ms)?;
    Ok((timed, id))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(value) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A media picture by id: the `p:pic` and its `p14:media`.
fn find_media(nodes: &[Node], id: u32) -> Result<(&Node, Option<&Node>)> {
    for pic in nodes.iter().filter(|n| n.is(1, "pic")) {
        let Some(nv) = child(nodes, pic, 1, "nvPicPr") else {
            continue;
        };
        let matches = child(nodes, nv, 1, "cNvPr")
            .and_then(|c| c.attr("id"))
            .and_then(|v| v.parse::<u32>().ok())
            == Some(id);
        if !matches {
            continue;
        }
        let Some(nv_pr) = child(nodes, nv, 1, "nvPr") else {
            continue;
        };
        let is_media = nv_pr.children.iter().any(|&i| {
            nodes[i].ns == 2
                && matches!(
                    nodes[i].name.as_str(),
                    "videoFile" | "audioFile" | "quickTimeFile" | "wavAudioFile"
                )
        });
        if !is_media {
            continue;
        }
        let p14 = super::objects::descendants(nodes, nv_pr)
            .iter()
            .find(|n| n.is(6, "media"));
        return Ok((pic, p14));
    }
    Err(WriteError::NoShape(id))
}

/// The trim, fades and bookmarks of a media picture.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaCuts {
    pub trim_start_ms: Option<f64>,
    pub trim_end_ms: Option<f64>,
    pub fade_in_ms: Option<f64>,
    pub fade_out_ms: Option<f64>,
    pub bookmarks: Vec<MediaBookmark>,
}

/// Rewrites a media picture's `p14:media` trim, fades and bookmarks,
/// keeping anything else it holds.
pub(crate) fn set_media_cuts(xml: &str, id: u32, cuts: &MediaCuts) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let (_, p14) = find_media(&nodes, id)?;
    let p14 = p14.ok_or(WriteError::UnsupportedObjectOperation(id))?;
    let positive = |value: Option<f64>| value.filter(|v| v.is_finite() && *v > 0.0);
    let mut bookmarks = cuts.bookmarks.clone();
    bookmarks.retain(|b| b.time_ms.is_finite() && b.time_ms >= 0.0);
    bookmarks.sort_by(|a, b| a.time_ms.total_cmp(&b.time_ms));
    let mut inner = p14_media_children(
        (positive(cuts.trim_start_ms), positive(cuts.trim_end_ms)),
        (positive(cuts.fade_in_ms), positive(cuts.fade_out_ms)),
        &bookmarks,
    );
    // Keep what this does not know (captions, extensions).
    for &index in &p14.children {
        let node = &nodes[index];
        if !(node.ns == 6 && matches!(node.name.as_str(), "trim" | "fade" | "bmkLst")) {
            inner.push_str(&xml[node.outer.clone()]);
        }
    }
    // The children use the p14 prefix; the element declares it if it
    // does not already have that binding.
    let opening = &xml[p14.opening.clone()];
    let declared = p14.qualified.starts_with("p14:")
        && (opening.contains("xmlns:p14=") || binds_p14(&nodes, p14));
    let open = if p14.empty {
        format!("{}>", opening.trim_end_matches("/>").trim_end())
    } else {
        opening.to_string()
    };
    let open = if declared {
        open
    } else {
        edit_tag(&open, &[("xmlns:p14", P14.to_string())])?
    };
    let replacement = if inner.is_empty() {
        format!("{}/>", open.trim_end_matches('>'))
    } else {
        format!("{open}{inner}</{}>", p14.qualified)
    };
    Ok(apply_patches(xml, vec![(p14.outer.clone(), replacement)]))
}

// ---- captions ----------------------------------------------------------------

const TRACKS_EXT_URI: &str = jade_pptx_reader::media::TRACKS_EXT_URI;
const P173: &str = jade_pptx_reader::media::P173_NAMESPACE;
const TRACK_RELATIONSHIP_TYPE: &str = jade_pptx_reader::media::TRACK_RELATIONSHIP_TYPE;

/// Where a caption track's WebVTT text comes from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CaptionSource<'a> {
    /// A `text/vtt` part in the package.
    Embed(&'a [u8]),
    /// A linked file (its URL or path, as written).
    Link(&'a str),
}

/// 字幕の挿入: a caption track for a media picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptionInsert<'a> {
    pub label: &'a str,
    pub lang: Option<&'a str>,
    pub source: CaptionSource<'a>,
}

fn prefix_of(qualified: &str) -> Option<&str> {
    qualified.split_once(':').map(|(prefix, _)| prefix)
}

/// The element named `local` (in any namespace) under `node`.
fn descendant<'a>(nodes: &'a [Node], node: &Node, local: &str) -> Option<&'a Node> {
    super::objects::descendants(nodes, node)
        .iter()
        .find(|n| n.name == local)
}

/// The `p:ext` holding a media's `tracksInfo`.
fn tracks_ext<'a>(nodes: &'a [Node], p14: &Node) -> Option<&'a Node> {
    super::objects::descendants(nodes, p14)
        .iter()
        .find(|n| n.name == "ext" && n.attr("uri") == Some(TRACKS_EXT_URI))
}

/// Adds a caption track to media picture `id`, as PowerPoint's 字幕の挿入
/// does ([MS-PPTX] 2.1.3, 2.13): the WebVTT part or link, the slide's
/// track relationship to it, and a `p173:track` in the media's
/// `tracksInfo`. The track's id comes back.
pub(crate) fn add_caption_track(
    package: &mut OpcPackage,
    slide_part: &str,
    xml: &str,
    id: u32,
    track: &CaptionInsert<'_>,
) -> Result<(String, String)> {
    let nodes = parse_nodes(xml)?;
    let (pic, p14) = find_media(&nodes, id)?;
    let p14 = p14.ok_or(WriteError::UnsupportedObjectOperation(id))?;
    let strict = nodes[0].strict;
    let audio = super::objects::descendants(&nodes, pic)
        .iter()
        .any(|n| n.ns == 2 && matches!(n.name.as_str(), "audioFile" | "wavAudioFile"));

    let mut parts = SlideParts::new(package, slide_part, &nodes)?;
    let (rel, attribute) = match track.source {
        CaptionSource::Embed(bytes) => {
            let part = parts.add_part("ppt/media", "media", "vtt", "text/vtt", bytes.to_vec())?;
            (
                parts.relate(TRACK_RELATIONSHIP_TYPE, &part, false)?,
                "embed",
            )
        }
        CaptionSource::Link(target) => {
            (parts.relate(TRACK_RELATIONSHIP_TYPE, target, true)?, "link")
        }
    };
    parts.finish();

    let guid = format!(
        "{{{}}}",
        uuid::Uuid::new_v4().to_string().to_ascii_uppercase()
    );
    let lang = track
        .lang
        .filter(|lang| !lang.is_empty())
        .map(|lang| format!(r#" lang="{}""#, escape(lang)))
        .unwrap_or_default();
    let element = format!(
        r#"<p173:track xmlns:p173="{P173}" xmlns:r="{}" id="{guid}" label="{}"{lang} r:{attribute}="{rel}"/>"#,
        r_ns(strict),
        escape(track.label),
    );

    let patch = if let Some(list) = descendant(&nodes, p14, "trackLst") {
        let at = if list.empty {
            list.outer.clone()
        } else {
            list.range.end..list.range.end
        };
        if list.empty {
            let open = xml[list.opening.clone()]
                .trim_end_matches("/>")
                .trim_end()
                .to_string();
            (at, format!("{open}>{element}</{}>", list.qualified))
        } else {
            (at, element)
        }
    } else if let Some(info) = descendant(&nodes, p14, "tracksInfo") {
        let list = format!(r#"<p173:trackLst xmlns:p173="{P173}">{element}</p173:trackLst>"#);
        if info.empty {
            let open = xml[info.opening.clone()]
                .trim_end_matches("/>")
                .trim_end()
                .to_string();
            (
                info.outer.clone(),
                format!("{open}>{list}</{}>", info.qualified),
            )
        } else {
            // trackLst is tracksInfo's first child.
            (info.opening.end..info.opening.end, list)
        }
    } else {
        let placement = if audio { "slide" } else { "media" };
        let ext = format!(
            r#"<p:ext xmlns:p="{}" uri="{TRACKS_EXT_URI}"><p173:tracksInfo xmlns:p173="{P173}" displayLoc="{placement}"><p173:trackLst>{element}</p173:trackLst></p173:tracksInfo></p:ext>"#,
            p_ns(strict)
        );
        let list = p14
            .children
            .iter()
            .map(|&i| &nodes[i])
            .find(|n| n.ns == 6 && n.name == "extLst");
        match list {
            Some(list) if !list.empty => (list.range.end..list.range.end, ext),
            Some(list) => {
                let open = xml[list.opening.clone()]
                    .trim_end_matches("/>")
                    .trim_end()
                    .to_string();
                (
                    list.outer.clone(),
                    format!("{open}>{ext}</{}>", list.qualified),
                )
            }
            None => {
                let prefix = prefix_of(&p14.qualified)
                    .map(|p| format!("{p}:"))
                    .unwrap_or_default();
                let list = format!("<{prefix}extLst>{ext}</{prefix}extLst>");
                if p14.empty {
                    let opening = &xml[p14.opening.clone()];
                    let open = opening.trim_end_matches("/>").trim_end();
                    (
                        p14.outer.clone(),
                        format!("{open}>{list}</{}>", p14.qualified),
                    )
                } else {
                    (p14.range.end..p14.range.end, list)
                }
            }
        }
    };
    Ok((apply_patches(xml, vec![patch]), guid))
}

/// 字幕の削除: takes caption track `track_id` off media picture `id`,
/// with its relationship and (embedded) part; the last track takes its
/// `tracksInfo` with it.
pub(crate) fn remove_caption_track(
    package: &mut OpcPackage,
    slide_part: &str,
    xml: &str,
    id: u32,
    track_id: &str,
) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let (_, p14) = find_media(&nodes, id)?;
    let p14 = p14.ok_or(WriteError::UnsupportedObjectOperation(id))?;
    let tracks: Vec<&Node> = super::objects::descendants(&nodes, p14)
        .iter()
        .filter(|n| n.name == "track")
        .collect();
    let Some(track) = tracks.iter().find(|n| n.attr("id") == Some(track_id)) else {
        return Err(WriteError::UnsupportedObjectOperation(id));
    };
    let rels: Vec<String> = track
        .attrs
        .iter()
        .filter(|(key, _)| key.ends_with(":embed") || key.ends_with(":link"))
        .map(|(_, value)| value.clone())
        .collect();
    let mut parts = SlideParts::new(package, slide_part, &nodes)?;
    for rel in &rels {
        parts.unrelate(rel)?;
    }
    parts.finish();
    let range = if tracks.len() > 1 {
        track.outer.clone()
    } else {
        let ext = tracks_ext(&nodes, p14).ok_or(WriteError::UnsupportedObjectOperation(id))?;
        // The extension list goes too when the tracks were all it held.
        let list = ext.parent.map(|i| &nodes[i]);
        match list {
            Some(list) if list.children.len() == 1 => list.outer.clone(),
            _ => ext.outer.clone(),
        }
    };
    Ok(apply_patches(xml, vec![(range, String::new())]))
}

fn binds_p14(nodes: &[Node], node: &Node) -> bool {
    let mut at = Some(node);
    while let Some(n) = at {
        if let Some(uri) = n.attr("xmlns:p14") {
            return uri == P14;
        }
        at = n.parent.map(|i| &nodes[i]);
    }
    false
}

/// Points a media picture's poster frame at a new picture.
pub(crate) fn set_poster(
    package: &mut OpcPackage,
    slide_part: &str,
    xml: &str,
    id: u32,
    image: &[u8],
) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let (pic, _) = find_media(&nodes, id)?;
    let blip = child(&nodes, pic, 1, "blipFill")
        .and_then(|fill| child(&nodes, fill, 2, "blip"))
        .ok_or(WriteError::UnsupportedObjectOperation(id))?;
    let key = blip
        .attrs
        .iter()
        .find(|(key, _)| key.contains(':') && key.ends_with(":embed"))
        .map(|(key, _)| key.clone());
    let mut parts = SlideParts::new(package, slide_part, &nodes)?;
    let rel = parts.add_image(image)?;
    parts.finish();
    let opening = &xml[blip.opening.clone()];
    let patched = match key {
        Some(key) => edit_tag(opening, &[(key.as_str(), rel)])?,
        None => {
            let declared = edit_tag(opening, &[("xmlns:jadeRel", r_ns(nodes[0].strict).into())])?;
            edit_tag(&declared, &[("jadeRel:embed", rel)])?
        }
    };
    Ok(apply_patches(xml, vec![(blip.opening.clone(), patched)]))
}

/// Puts new media behind a media picture (compressed, converted or
/// embedded from its link), keeping its poster and timing.
pub(crate) fn replace_media(
    package: &mut OpcPackage,
    slide_part: &str,
    xml: &str,
    id: u32,
    kind: MediaKind,
    source: &MediaSource<'_>,
) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let (pic, p14) = find_media(&nodes, id)?;
    let nv_pr = child(&nodes, pic, 1, "nvPicPr")
        .and_then(|nv| child(&nodes, nv, 1, "nvPr"))
        .ok_or(WriteError::NoShape(id))?;
    let file = nv_pr
        .children
        .iter()
        .map(|&i| &nodes[i])
        .find(|n| {
            n.ns == 2
                && matches!(
                    n.name.as_str(),
                    "videoFile" | "audioFile" | "quickTimeFile" | "wavAudioFile"
                )
        })
        .ok_or(WriteError::NoShape(id))?;
    let mut parts = SlideParts::new(package, slide_part, &nodes)?;
    let (media_rel, file_rel, linked) = media_relationships(&mut parts, kind, source)?;
    parts.finish();
    let mut patches = Vec::new();
    let file_key = file
        .attrs
        .iter()
        .find(|(key, _)| key.contains(':') && (key.ends_with(":link") || key.ends_with(":embed")))
        .map(|(key, _)| key.clone())
        .ok_or(WriteError::UnsupportedObjectOperation(id))?;
    let file_key = file_key.replace(":embed", ":link");
    let opening = super::remove_tag_attributes(
        &xml[file.opening.clone()],
        &file
            .attrs
            .iter()
            .filter(|(key, _)| key.ends_with(":embed") || key.ends_with(":link"))
            .map(|(key, _)| key.as_str())
            .collect::<Vec<_>>(),
    )?;
    patches.push((
        file.opening.clone(),
        edit_tag(&opening, &[(file_key.as_str(), file_rel)])?,
    ));
    if let Some(p14) = p14 {
        let prefix = p14
            .attrs
            .iter()
            .find(|(key, _)| key.ends_with(":embed") || key.ends_with(":link"))
            .and_then(|(key, _)| key.split_once(':').map(|(prefix, _)| prefix.to_string()))
            .unwrap_or_else(|| file_key.split_once(':').unwrap().0.to_string());
        let names: Vec<String> = p14
            .attrs
            .iter()
            .filter(|(key, _)| key.ends_with(":embed") || key.ends_with(":link"))
            .map(|(key, _)| key.clone())
            .collect();
        let stripped = super::remove_tag_attributes(
            &xml[p14.opening.clone()],
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        )?;
        let attr = format!("{prefix}:{}", if linked { "link" } else { "embed" });
        patches.push((
            p14.opening.clone(),
            edit_tag(&stripped, &[(attr.as_str(), media_rel)])?,
        ));
    }
    Ok(apply_patches(xml, patches))
}

// ---- timing ----------------------------------------------------------------

fn spid_of(nodes: &[Node], node: &Node) -> Option<u32> {
    super::objects::descendants(nodes, node)
        .iter()
        .find(|n| n.is(1, "spTgt"))
        .and_then(|n| n.attr("spid"))
        .and_then(|v| v.parse().ok())
}

fn c_tn<'a>(nodes: &'a [Node], node: &Node) -> Option<&'a Node> {
    child(nodes, node, 1, "cTn")
}

fn child_list<'a>(nodes: &'a [Node], node: &Node) -> Option<&'a Node> {
    c_tn(nodes, node).and_then(|c| child(nodes, c, 1, "childTnLst"))
}

/// The time nodes that play media `id`: its media node and the play,
/// pause and stop effects aimed at it, with what they would leave empty.
fn media_timing_ranges(nodes: &[Node], id: u32) -> Vec<std::ops::Range<usize>> {
    let Some(timing) = child(nodes, &nodes[0], 1, "timing") else {
        return Vec::new();
    };
    let within = super::objects::descendants(nodes, timing);
    let start = nodes.partition_point(|n| n.outer.start < timing.outer.start);
    let mut removed = vec![false; nodes.len()];
    for (offset, node) in within.iter().enumerate() {
        let index = start + offset;
        let media_node = node.ns == 1
            && matches!(node.name.as_str(), "video" | "audio")
            && child(nodes, node, 1, "cMediaNode").and_then(|m| spid_of(nodes, m)) == Some(id);
        let media_effect = node.is(1, "par")
            && c_tn(nodes, node).is_some_and(|c| c.attr("presetClass") == Some("mediacall"))
            && spid_of(nodes, node) == Some(id);
        if media_node || media_effect {
            removed[index] = true;
        }
    }
    // A par or seq left with nothing to run goes too.
    loop {
        let mut changed = false;
        for (offset, node) in within.iter().enumerate() {
            let index = start + offset;
            if removed[index] || !(node.is(1, "par") || node.is(1, "seq")) {
                continue;
            }
            let Some(list) = child_list(nodes, node) else {
                continue;
            };
            if c_tn(nodes, node).and_then(|c| c.attr("nodeType")) == Some("tmRoot") {
                continue;
            }
            if !list.children.is_empty() && list.children.iter().all(|&i| removed[i]) {
                removed[index] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        if removed[index]
            && !ranges
                .iter()
                .any(|r| r.start <= node.outer.start && r.end >= node.outer.end)
        {
            ranges.push(node.outer.clone());
        }
    }
    // Nothing left under the root: the whole timing goes.
    let root_list = child(nodes, timing, 1, "tnLst")
        .and_then(|list| child(nodes, list, 1, "par"))
        .and_then(|par| child_list(nodes, par));
    if let Some(list) = root_list {
        let left = list.children.iter().any(|&i| {
            !ranges
                .iter()
                .any(|r| r.start <= nodes[i].outer.start && r.end >= nodes[i].outer.end)
        });
        if !left && child(nodes, timing, 1, "bldLst").is_none() {
            return vec![timing.outer.clone()];
        }
    }
    ranges
}

/// Takes media `id` out of the slide's timing.
pub(crate) fn remove_playback(xml: &str, id: u32) -> Result<String> {
    let nodes = parse_nodes(xml)?;
    let ranges = media_timing_ranges(&nodes, id);
    Ok(apply_patches(
        xml,
        ranges.into_iter().map(|r| (r, String::new())).collect(),
    ))
}

struct Ids(u32);
impl Ids {
    fn next(&mut self) -> u32 {
        self.0 += 1;
        self.0
    }
}

/// A click group in the main sequence that plays media `spid`.
fn play_group(
    ids: &mut Ids,
    spid: u32,
    auto_after: Option<u32>,
    duration_ms: Option<f64>,
) -> String {
    let (conditions, node_type) = match auto_after {
        Some(main) => (
            format!(
                r#"<p:cond delay="indefinite"/><p:cond evt="onBegin" delay="0"><p:tn val="{main}"/></p:cond>"#
            ),
            "afterEffect",
        ),
        None => (r#"<p:cond delay="indefinite"/>"#.to_string(), "clickEffect"),
    };
    let dur = duration_ms
        .filter(|ms| *ms >= 1.0)
        .map(|ms| format!(r#" dur="{}""#, ms.round() as u64))
        .unwrap_or_default();
    let (a, b, c, d) = (ids.next(), ids.next(), ids.next(), ids.next());
    format!(
        r#"<p:par><p:cTn id="{a}" fill="hold"><p:stCondLst>{conditions}</p:stCondLst><p:childTnLst><p:par><p:cTn id="{b}" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="{c}" presetID="1" presetClass="mediacall" presetSubtype="0" fill="hold" nodeType="{node_type}"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:cmd type="call" cmd="playFrom(0.0)"><p:cBhvr><p:cTn id="{d}"{dur} fill="hold"/><p:tgtEl><p:spTgt spid="{spid}"/></p:tgtEl></p:cBhvr></p:cmd></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par>"#
    )
}

/// An interactive sequence: a click on media `spid` plays or pauses it.
fn toggle_sequence(ids: &mut Ids, spid: u32) -> String {
    let (a, b, c, d, e) = (ids.next(), ids.next(), ids.next(), ids.next(), ids.next());
    format!(
        r#"<p:seq concurrent="1" nextAc="seek"><p:cTn id="{a}" restart="whenNotActive" fill="hold" evtFilter="cancelBubble" nodeType="interactiveSeq"><p:stCondLst><p:cond evt="onClick" delay="0"><p:tgtEl><p:spTgt spid="{spid}"/></p:tgtEl></p:cond></p:stCondLst><p:endSync evt="end" delay="0"><p:rtn val="all"/></p:endSync><p:childTnLst><p:par><p:cTn id="{b}" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="{c}" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:par><p:cTn id="{d}" presetID="2" presetClass="mediacall" presetSubtype="0" fill="hold" nodeType="clickEffect"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst><p:cmd type="call" cmd="togglePause"><p:cBhvr><p:cTn id="{e}" dur="1" fill="hold"/><p:tgtEl><p:spTgt spid="{spid}"/></p:tgtEl></p:cBhvr></p:cmd></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn><p:nextCondLst><p:cond evt="onClick" delay="0"><p:tgtEl><p:spTgt spid="{spid}"/></p:tgtEl></p:cond></p:nextCondLst></p:seq>"#
    )
}

/// The media node: volume, looping, hiding, slides to play across.
fn media_node(ids: &mut Ids, playback: &MediaPlayback) -> String {
    let element = match playback.kind {
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
    };
    let mut node_attrs = format!(r#" vol="{}""#, playback.volume.min(100_000));
    if playback.mute {
        node_attrs.push_str(r#" mute="1""#);
    }
    if playback.slides > 1 {
        node_attrs.push_str(&format!(r#" numSld="{}""#, playback.slides));
    }
    if playback.hide_when_not_playing {
        node_attrs.push_str(r#" showWhenStopped="0""#);
    }
    let repeat = if playback.loop_until_stopped {
        r#" repeatCount="indefinite""#
    } else {
        ""
    };
    let fill = if playback.rewind { "remove" } else { "hold" };
    let end = if playback.kind == MediaKind::Audio {
        r#"<p:endCondLst><p:cond evt="onStopAudio" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:endCondLst>"#
    } else {
        ""
    };
    let full = if playback.full_screen && playback.kind == MediaKind::Video {
        r#" fullScrn="1""#
    } else {
        ""
    };
    format!(
        r#"<p:{element}{full}><p:cMediaNode{node_attrs}><p:cTn id="{}"{repeat} fill="{fill}" display="0"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst>{end}</p:cTn><p:tgtEl><p:spTgt spid="{}"/></p:tgtEl></p:cMediaNode></p:{element}>"#,
        ids.next(),
        playback.shape_id
    )
}

/// Wraps `fragment` (unprefixed `p:` markup) so it reads as PresentationML
/// wherever it lands.
fn declared(fragment: &str, strict: bool) -> String {
    // Each top-level element declares `p`; `fragment` may hold several.
    let ns = format!(" xmlns:p=\"{}\"", p_ns(strict));
    let mut out = String::new();
    let mut depth = 0i32;
    let bytes = fragment.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let closing = bytes.get(i + 1) == Some(&b'/');
            let end = fragment[i..]
                .find('>')
                .map(|e| i + e)
                .unwrap_or(bytes.len() - 1);
            let self_closing = bytes[end - 1] == b'/';
            if !closing && depth == 0 {
                let name_end = fragment[i + 1..]
                    .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                    .map(|e| i + 1 + e)
                    .unwrap();
                out.push_str(&fragment[i..name_end]);
                out.push_str(&ns);
                out.push_str(&fragment[name_end..=end]);
            } else {
                out.push_str(&fragment[i..=end]);
            }
            if closing {
                depth -= 1;
            } else if !self_closing {
                depth += 1;
            }
            i = end + 1;
        } else {
            let next = fragment[i..]
                .find('<')
                .map(|e| i + e)
                .unwrap_or(bytes.len());
            out.push_str(&fragment[i..next]);
            i = next;
        }
    }
    out
}

/// Writes how media plays: its media node and the effects that start it,
/// replacing whatever the timing had for it, leaving the rest alone.
pub(crate) fn set_playback(
    xml: &str,
    playback: &MediaPlayback,
    duration_ms: Option<f64>,
) -> Result<String> {
    let spid = playback.shape_id;
    let cleared = remove_playback(xml, spid)?;
    let nodes = parse_nodes(&cleared)?;
    let strict = nodes[0].strict;
    let root = &nodes[0];
    let timing = child(&nodes, root, 1, "timing");
    let max_id = timing
        .map(|timing| {
            super::objects::descendants(&nodes, timing)
                .iter()
                .filter(|n| n.is(1, "cTn"))
                .filter_map(|n| n.attr("id").and_then(|v| v.parse::<u32>().ok()))
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    let mut ids = Ids(max_id.max(1));
    let media = media_node(&mut ids, playback);
    let interactive = match (playback.start, playback.kind) {
        (MediaStart::WhenClicked, _) | (MediaStart::InClickSequence, MediaKind::Video) => {
            toggle_sequence(&mut ids, spid)
        }
        _ => String::new(),
    };
    let root_list = timing
        .and_then(|timing| child(&nodes, timing, 1, "tnLst"))
        .and_then(|list| child(&nodes, list, 1, "par"))
        .and_then(|par| child_list(&nodes, par));
    let Some(root_list) = root_list else {
        // No timing yet: write one.
        let mut ids = Ids(1);
        let main_id = 2;
        ids.0 = 2;
        let group = match playback.start {
            MediaStart::WhenClicked => None,
            MediaStart::InClickSequence => Some(play_group(&mut ids, spid, None, duration_ms)),
            MediaStart::Automatically => {
                Some(play_group(&mut ids, spid, Some(main_id), duration_ms))
            }
        };
        let main = group
            .map(|group| {
                format!(r#"<p:seq concurrent="1" nextAc="seek"><p:cTn id="{main_id}" dur="indefinite" nodeType="mainSeq"><p:childTnLst>{group}</p:childTnLst></p:cTn><p:prevCondLst><p:cond evt="onPrev" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:prevCondLst><p:nextCondLst><p:cond evt="onNext" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:nextCondLst></p:seq>"#)
            })
            .unwrap_or_default();
        let media = media_node(&mut ids, playback);
        let interactive = match (playback.start, playback.kind) {
            (MediaStart::WhenClicked, _) | (MediaStart::InClickSequence, MediaKind::Video) => {
                toggle_sequence(&mut ids, spid)
            }
            _ => String::new(),
        };
        let timing = declared(
            &format!(
                r#"<p:timing><p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst>{main}{media}{interactive}</p:childTnLst></p:cTn></p:par></p:tnLst></p:timing>"#
            ),
            strict,
        );
        // After the transition, before the slide's extensions.
        let at = child(&nodes, root, 1, "extLst").map_or(root.range.end, |n| n.outer.start);
        return Ok(apply_patches(&cleared, vec![(at..at, timing)]));
    };
    let mut patches = Vec::new();
    let main = root_list.children.iter().map(|&i| &nodes[i]).find(|n| {
        n.is(1, "seq") && c_tn(&nodes, n).and_then(|c| c.attr("nodeType")) == Some("mainSeq")
    });
    let group_for = |ids: &mut Ids, main_id: Option<u32>| match playback.start {
        MediaStart::WhenClicked => None,
        MediaStart::InClickSequence => Some(play_group(ids, spid, None, duration_ms)),
        MediaStart::Automatically => Some(play_group(ids, spid, main_id, duration_ms)),
    };
    match main {
        Some(main) => {
            let main_id = c_tn(&nodes, main)
                .and_then(|c| c.attr("id"))
                .and_then(|v| v.parse().ok());
            if let (Some(group), Some(list)) =
                (group_for(&mut ids, main_id), child_list(&nodes, main))
            {
                let group = declared(&group, strict);
                if playback.start == MediaStart::Automatically {
                    // First, so it plays as the slide comes up.
                    let at = list
                        .children
                        .first()
                        .map_or(list.range.end, |&i| nodes[i].outer.start);
                    patches.push((at..at, group));
                } else {
                    patches.push((list.range.end..list.range.end, group));
                }
            }
        }
        None => {
            let main_id = ids.next();
            if let Some(group) = group_for(&mut ids, Some(main_id)) {
                let seq = declared(
                    &format!(
                        r#"<p:seq concurrent="1" nextAc="seek"><p:cTn id="{main_id}" dur="indefinite" nodeType="mainSeq"><p:childTnLst>{group}</p:childTnLst></p:cTn><p:prevCondLst><p:cond evt="onPrev" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:prevCondLst><p:nextCondLst><p:cond evt="onNext" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:nextCondLst></p:seq>"#
                    ),
                    strict,
                );
                let at = root_list.range.start;
                patches.push((at..at, seq));
            }
        }
    }
    let tail = declared(&format!("{media}{interactive}"), strict);
    if root_list.empty {
        return Err(WriteError::Xml("empty timing root".into()));
    }
    patches.push((root_list.range.end..root_list.range.end, tail));
    Ok(apply_patches(&cleared, patches))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_puts_the_namespace_on_each_top_level_element() {
        let out = declared("<p:a x=\"1\"><p:b/></p:a><p:c/>", false);
        assert_eq!(
            out,
            format!(
                "<p:a xmlns:p=\"{0}\" x=\"1\"><p:b/></p:a><p:c xmlns:p=\"{0}\"/>",
                p_ns(false)
            )
        );
    }

    #[test]
    fn percent_escapes_in_linked_names_are_decoded() {
        assert_eq!(percent_decode("My%20Movie.mp4"), "My Movie.mp4");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
