//! pptx (OOXML パッケージ) を読んで [`PresentationIr`] にする。
//!
//! プレースホルダは位置を持たず、スライドレイアウト → スライドマスタの順に
//! 継承する。描画に使う IR なので、ここで継承を解決してから返す。

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Seek};

use anyhow::{anyhow, Context, Result};
use base64::Engine as _;
use zip::ZipArchive;

use crate::ir::{
    content_hash, FormatSpanIr, Frame, ImageIr, ParagraphIr, Placeholder, PresentationIr, ShapeIr,
    ShapeKind, SlideIr, TableIr,
};
use crate::xml::{self, Node};

/// 読み込み時の上限。壊れた / 悪意のあるファイルで潰れないようにする。
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_input_bytes: u64,
    pub max_uncompressed_bytes: u64,
    pub max_zip_entries: usize,
    pub max_slides: usize,
    pub max_shapes_per_slide: usize,
    /// data URL にする画像 1 枚あたりの上限。超えたら埋め込まない。
    pub max_image_bytes: usize,
    /// 埋め込む画像の合計上限。
    pub max_media_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_bytes: 128 * 1024 * 1024,
            max_uncompressed_bytes: 512 * 1024 * 1024,
            max_zip_entries: 20_000,
            max_slides: 2_000,
            max_shapes_per_slide: 5_000,
            max_image_bytes: 8 * 1024 * 1024,
            max_media_bytes: 64 * 1024 * 1024,
        }
    }
}

/// zip の中身をパート名で引くためのラッパ。
struct Package<R: Read + Seek> {
    zip: ZipArchive<R>,
}

impl<R: Read + Seek> Package<R> {
    fn open(reader: R, input_size: u64, limits: &Limits) -> Result<Self> {
        anyhow::ensure!(
            input_size <= limits.max_input_bytes,
            "pptx input is {input_size} bytes, exceeding max_input_bytes {}",
            limits.max_input_bytes
        );
        let mut zip = ZipArchive::new(reader).context("failed to open pptx as a ZIP container")?;
        anyhow::ensure!(
            zip.len() <= limits.max_zip_entries,
            "pptx ZIP contains {} entries, exceeding max_zip_entries {}",
            zip.len(),
            limits.max_zip_entries
        );
        let mut uncompressed = 0u64;
        for index in 0..zip.len() {
            let entry = zip.by_index_raw(index)?;
            uncompressed = uncompressed.saturating_add(entry.size());
            anyhow::ensure!(
                uncompressed <= limits.max_uncompressed_bytes,
                "pptx expands to more than max_uncompressed_bytes {}",
                limits.max_uncompressed_bytes
            );
        }
        Ok(Self { zip })
    }

    fn read(&mut self, part: &str) -> Result<Vec<u8>> {
        let mut file = self
            .zip
            .by_name(part)
            .with_context(|| format!("missing pptx part: {part}"))?;
        let mut bytes = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut bytes)
            .with_context(|| format!("failed to read pptx part: {part}"))?;
        Ok(bytes)
    }

    fn try_read(&mut self, part: &str) -> Option<Vec<u8>> {
        self.read(part).ok()
    }

    fn xml(&mut self, part: &str) -> Result<Node> {
        let bytes = self.read(part)?;
        xml::parse(&bytes).with_context(|| format!("failed to parse xml part: {part}"))
    }

    fn try_xml(&mut self, part: &str) -> Option<Node> {
        let bytes = self.try_read(part)?;
        xml::parse(&bytes).ok()
    }

    /// `part` に対応する `_rels` を読み、rId → 解決済みパート名にする。
    fn rels(&mut self, part: &str) -> BTreeMap<String, String> {
        let (dir, file) = split_part(part);
        let rels_part = if dir.is_empty() {
            format!("_rels/{file}.rels")
        } else {
            format!("{dir}/_rels/{file}.rels")
        };
        let Some(root) = self.try_xml(&rels_part) else {
            return BTreeMap::new();
        };
        let mut out = BTreeMap::new();
        for rel in root.children_named("Relationship") {
            let (Some(id), Some(target)) = (rel.attr("Id"), rel.attr("Target")) else {
                continue;
            };
            // 外部リンクは実体が無いので飛ばす。
            if rel.attr("TargetMode") == Some("External") {
                continue;
            }
            out.insert(id.to_string(), resolve_target(dir, target));
        }
        out
    }
}

fn split_part(part: &str) -> (&str, &str) {
    match part.rsplit_once('/') {
        Some((dir, file)) => (dir, file),
        None => ("", part),
    }
}

/// `ppt/slides` から見た `../media/image1.png` を `ppt/media/image1.png` にする。
fn resolve_target(base_dir: &str, target: &str) -> String {
    let target = target.trim_start_matches('/');
    if base_dir.is_empty() {
        return target.to_string();
    }
    let mut segments: Vec<&str> = base_dir.split('/').collect();
    for piece in target.split('/') {
        match piece {
            "." | "" => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    segments.join("/")
}

#[derive(Debug, Clone)]
struct PptxTheme {
    colors: BTreeMap<String, String>,
    aliases: BTreeMap<String, String>,
}

impl Default for PptxTheme {
    fn default() -> Self {
        let aliases = [
            ("bg1", "lt1"),
            ("tx1", "dk1"),
            ("bg2", "lt2"),
            ("tx2", "dk2"),
            ("accent1", "accent1"),
            ("accent2", "accent2"),
            ("accent3", "accent3"),
            ("accent4", "accent4"),
            ("accent5", "accent5"),
            ("accent6", "accent6"),
            ("hlink", "hlink"),
            ("folHlink", "folHlink"),
        ]
        .into_iter()
        .map(|(alias, color)| (alias.to_string(), color.to_string()))
        .collect();
        Self {
            colors: BTreeMap::new(),
            aliases,
        }
    }
}

impl PptxTheme {
    fn merge_theme(&mut self, root: &Node) {
        let Some(color_scheme) = root
            .descendants()
            .into_iter()
            .find(|node| node.name == "clrScheme")
        else {
            return;
        };
        for color in &color_scheme.children {
            let Some(value) = color.children.first() else {
                continue;
            };
            self.colors
                .insert(color.name.clone(), node_signature(value));
        }
    }

    fn apply_color_mapping(&mut self, root: &Node, mapping_name: &str) {
        let Some(mapping) = root
            .descendants()
            .into_iter()
            .find(|node| node.name == mapping_name)
        else {
            return;
        };
        for (name, value) in &mapping.attrs {
            let name = name.rsplit(':').next().unwrap_or(name);
            if self.aliases.contains_key(name) {
                self.aliases.insert(name.to_string(), value.to_string());
            }
        }
    }

    fn resolve(&self, scheme_color: &str) -> Option<&str> {
        let color = self
            .aliases
            .get(scheme_color)
            .map_or(scheme_color, String::as_str);
        self.colors.get(color).map(String::as_str)
    }
}

/// pptx のバイト列を読み込む。
pub fn load_presentation(bytes: &[u8], limits: &Limits) -> Result<PresentationIr> {
    let mut package = Package::open(Cursor::new(bytes), bytes.len() as u64, limits)?;

    let presentation = package.xml("ppt/presentation.xml")?;
    let size = presentation.child("sldSz");
    let width = size
        .and_then(|node| node.attr_i64("cx"))
        .unwrap_or(9_144_000);
    let height = size
        .and_then(|node| node.attr_i64("cy"))
        .unwrap_or(6_858_000);

    let rels = package.rels("ppt/presentation.xml");
    let mut slide_parts = Vec::new();
    if let Some(list) = presentation.child("sldIdLst") {
        for entry in list.children_named("sldId") {
            if let Some(part) = entry.attr("r:id").and_then(|rid| rels.get(rid)) {
                slide_parts.push(part.clone());
            }
        }
    }
    // sldIdLst から引けなければ、パート名の順で拾う。
    if slide_parts.is_empty() {
        let mut names: Vec<String> = (0..package.zip.len())
            .filter_map(|i| {
                package
                    .zip
                    .by_index_raw(i)
                    .ok()
                    .map(|f| f.name().to_string())
            })
            .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
            .collect();
        names.sort_by_key(|name| slide_number(name));
        slide_parts = names;
    }
    anyhow::ensure!(
        slide_parts.len() <= limits.max_slides,
        "presentation has {} slides, exceeding max_slides {}",
        slide_parts.len(),
        limits.max_slides
    );

    let mut media = BTreeMap::new();
    let mut media_bytes = 0usize;
    let mut slides = Vec::with_capacity(slide_parts.len());
    for (position, part) in slide_parts.iter().enumerate() {
        let slide = parse_slide(
            &mut package,
            part,
            position + 1,
            limits,
            &mut media,
            &mut media_bytes,
        )
        .with_context(|| format!("failed to parse slide: {part}"))?;
        slides.push(slide);
    }

    Ok(PresentationIr {
        width,
        height,
        slides,
        media,
    })
}

fn slide_number(name: &str) -> usize {
    name.trim_start_matches("ppt/slides/slide")
        .trim_end_matches(".xml")
        .parse()
        .unwrap_or(usize::MAX)
}

fn parse_slide<R: Read + Seek>(
    package: &mut Package<R>,
    part: &str,
    index: usize,
    limits: &Limits,
    media: &mut BTreeMap<String, String>,
    media_bytes: &mut usize,
) -> Result<SlideIr> {
    let root = package.xml(part)?;
    let rels = package.rels(part);

    // プレースホルダの位置はレイアウト → マスタの順に継承する。
    let layout_part = rels
        .values()
        .find(|target| target.contains("slideLayouts/"))
        .cloned();
    let layout = layout_part.as_deref().and_then(|p| package.try_xml(p));
    let layout_rels = layout_part
        .as_deref()
        .map(|part| package.rels(part))
        .unwrap_or_default();
    let master_part = layout_rels
        .values()
        .find(|target| target.contains("slideMasters/"))
        .cloned();
    let master = master_part.as_deref().and_then(|p| package.try_xml(p));
    let master_rels = master_part
        .as_deref()
        .map(|part| package.rels(part))
        .unwrap_or_default();

    let mut theme = PptxTheme::default();
    for relationships in [&master_rels, &layout_rels, &rels] {
        for theme_part in relationships
            .values()
            .filter(|target| target.contains("/theme/") && target.ends_with(".xml"))
        {
            if let Some(theme_root) = package.try_xml(theme_part) {
                theme.merge_theme(&theme_root);
            }
        }
    }
    if let Some(master) = master.as_ref() {
        theme.apply_color_mapping(master, "clrMap");
    }
    if let Some(layout) = layout.as_ref() {
        theme.apply_color_mapping(layout, "overrideClrMapping");
    }
    theme.apply_color_mapping(&root, "overrideClrMapping");

    let inherited = [layout.as_ref(), master.as_ref()]
        .into_iter()
        .flatten()
        .map(placeholder_frames)
        .collect::<Vec<_>>();

    let tree = root
        .path(&["cSld", "spTree"])
        .ok_or_else(|| anyhow!("slide has no spTree"))?;

    let mut shapes = Vec::new();
    collect_shapes(
        tree,
        None,
        &rels,
        &inherited,
        package,
        limits,
        media,
        media_bytes,
        &theme,
        &mut shapes,
    );
    anyhow::ensure!(
        shapes.len() <= limits.max_shapes_per_slide,
        "slide {index} has {} shapes, exceeding max_shapes_per_slide {}",
        shapes.len(),
        limits.max_shapes_per_slide
    );

    let notes = rels
        .values()
        .find(|target| target.contains("notesSlides/"))
        .and_then(|target| package.try_xml(target))
        .map(|node| {
            node.descendants()
                .iter()
                .filter(|child| child.name == "p")
                .map(|paragraph| paragraph.text_content())
                .filter(|text| !text.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.trim().is_empty());

    Ok(SlideIr {
        part: part.to_string(),
        index,
        shapes,
        notes,
    })
}

/// レイアウト / マスタから「プレースホルダ種別 → 位置」を集める。
fn placeholder_frames(root: &Node) -> Vec<(Placeholder, Frame)> {
    let Some(tree) = root.path(&["cSld", "spTree"]) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for shape in tree.descendants() {
        if shape.name != "sp" {
            continue;
        }
        let Some(placeholder) = read_placeholder(shape) else {
            continue;
        };
        if let Some(frame) = read_frame(shape.path(&["spPr", "xfrm"])) {
            out.push((placeholder, frame));
        }
    }
    out
}

fn read_placeholder(shape: &Node) -> Option<Placeholder> {
    let ph = shape.path(&["nvSpPr", "nvPr", "ph"])?;
    Some(Placeholder {
        // type 省略時の既定は body (OOXML)。
        kind: ph.attr("type").unwrap_or("body").to_string(),
        index: ph.attr("idx").and_then(|value| value.parse().ok()),
    })
}

fn read_frame(xfrm: Option<&Node>) -> Option<Frame> {
    let xfrm = xfrm?;
    let off = xfrm.child("off")?;
    let ext = xfrm.child("ext")?;
    Some(Frame {
        x: off.attr_i64("x")?,
        y: off.attr_i64("y")?,
        width: ext.attr_i64("cx")?,
        height: ext.attr_i64("cy")?,
    })
}

/// 継承元から一致するプレースホルダの位置を探す。
///
/// idx が一致するものを最優先し、無ければ種別が一致するものを使う。
fn inherit_frame(
    placeholder: &Placeholder,
    inherited: &[Vec<(Placeholder, Frame)>],
) -> Option<Frame> {
    for level in inherited {
        if let Some(index) = placeholder.index {
            if let Some((_, frame)) = level
                .iter()
                .find(|(candidate, _)| candidate.index == Some(index))
            {
                return Some(*frame);
            }
        }
        if let Some((_, frame)) = level.iter().find(|(candidate, _)| {
            candidate.kind == placeholder.kind
                || (is_title(&candidate.kind) && is_title(&placeholder.kind))
        }) {
            return Some(*frame);
        }
    }
    None
}

fn is_title(kind: &str) -> bool {
    kind == "title" || kind == "ctrTitle"
}

/// グループの子座標をスライド座標へ移す線形変換。
#[derive(Debug, Clone, Copy)]
struct GroupTransform {
    off_x: i64,
    off_y: i64,
    child_x: i64,
    child_y: i64,
    scale_x: f64,
    scale_y: f64,
}

impl GroupTransform {
    fn apply(&self, frame: Frame) -> Frame {
        Frame {
            x: self.off_x + ((frame.x - self.child_x) as f64 * self.scale_x) as i64,
            y: self.off_y + ((frame.y - self.child_y) as f64 * self.scale_y) as i64,
            width: (frame.width as f64 * self.scale_x) as i64,
            height: (frame.height as f64 * self.scale_y) as i64,
        }
    }

    fn compose(outer: Option<GroupTransform>, inner: GroupTransform) -> GroupTransform {
        match outer {
            None => inner,
            Some(outer) => {
                let origin = outer.apply(Frame {
                    x: inner.off_x,
                    y: inner.off_y,
                    width: 0,
                    height: 0,
                });
                GroupTransform {
                    off_x: origin.x,
                    off_y: origin.y,
                    child_x: inner.child_x,
                    child_y: inner.child_y,
                    scale_x: inner.scale_x * outer.scale_x,
                    scale_y: inner.scale_y * outer.scale_y,
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_shapes<R: Read + Seek>(
    tree: &Node,
    transform: Option<GroupTransform>,
    rels: &BTreeMap<String, String>,
    inherited: &[Vec<(Placeholder, Frame)>],
    package: &mut Package<R>,
    limits: &Limits,
    media: &mut BTreeMap<String, String>,
    media_bytes: &mut usize,
    theme: &PptxTheme,
    out: &mut Vec<ShapeIr>,
) {
    for child in &tree.children {
        match child.name.as_str() {
            "sp" => {
                if let Some(shape) = parse_sp(child, transform, inherited, theme, out.len()) {
                    out.push(shape);
                }
            }
            "pic" => {
                if let Some(shape) = parse_pic(
                    child,
                    transform,
                    rels,
                    package,
                    limits,
                    media,
                    media_bytes,
                    theme,
                    out.len(),
                ) {
                    out.push(shape);
                }
            }
            "graphicFrame" => {
                let mut related_paragraphs = read_chart_paragraphs(child, rels, package);
                if related_paragraphs.is_empty() {
                    related_paragraphs = read_diagram_paragraphs(child, rels, package, theme);
                }
                if let Some(shape) =
                    parse_graphic_frame(child, transform, related_paragraphs, theme, out.len())
                {
                    out.push(shape);
                }
            }
            "grpSp" => {
                let inner = child
                    .path(&["grpSpPr", "xfrm"])
                    .and_then(group_transform)
                    .map(|inner| GroupTransform::compose(transform, inner));
                collect_shapes(
                    child,
                    inner.or(transform),
                    rels,
                    inherited,
                    package,
                    limits,
                    media,
                    media_bytes,
                    theme,
                    out,
                );
            }
            _ => {}
        }
    }
}

fn group_transform(xfrm: &Node) -> Option<GroupTransform> {
    let off = xfrm.child("off")?;
    let ext = xfrm.child("ext")?;
    let child_off = xfrm.child("chOff")?;
    let child_ext = xfrm.child("chExt")?;
    let child_cx = child_ext.attr_i64("cx").unwrap_or(0);
    let child_cy = child_ext.attr_i64("cy").unwrap_or(0);
    Some(GroupTransform {
        off_x: off.attr_i64("x")?,
        off_y: off.attr_i64("y")?,
        child_x: child_off.attr_i64("x").unwrap_or(0),
        child_y: child_off.attr_i64("y").unwrap_or(0),
        scale_x: if child_cx == 0 {
            1.0
        } else {
            ext.attr_i64("cx").unwrap_or(child_cx) as f64 / child_cx as f64
        },
        scale_y: if child_cy == 0 {
            1.0
        } else {
            ext.attr_i64("cy").unwrap_or(child_cy) as f64 / child_cy as f64
        },
    })
}

fn shape_identity(node: &Node, container: &str) -> (String, String) {
    let props = node.path(&[container, "cNvPr"]);
    (
        props
            .and_then(|p| p.attr("id"))
            .unwrap_or_default()
            .to_string(),
        props
            .and_then(|p| p.attr("name"))
            .unwrap_or_default()
            .to_string(),
    )
}

fn parse_sp(
    node: &Node,
    transform: Option<GroupTransform>,
    inherited: &[Vec<(Placeholder, Frame)>],
    theme: &PptxTheme,
    z: usize,
) -> Option<ShapeIr> {
    let (id, name) = shape_identity(node, "nvSpPr");
    let placeholder = read_placeholder(node);
    let own = read_frame(node.path(&["spPr", "xfrm"]));
    let frame = own
        .or_else(|| {
            placeholder
                .as_ref()
                .and_then(|ph| inherit_frame(ph, inherited))
        })
        .map(|frame| match transform {
            Some(transform) if own.is_some() => transform.apply(frame),
            _ => frame,
        });

    let paragraphs = node
        .child("txBody")
        .map(|body| read_paragraphs_with_theme(body, Some(theme)))
        .unwrap_or_default();

    Some(ShapeIr {
        id,
        name,
        kind: ShapeKind::Text,
        placeholder,
        frame,
        z,
        appearance_signature: shape_appearance_signature(node, theme),
        paragraphs,
        table: None,
        image: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn parse_pic<R: Read + Seek>(
    node: &Node,
    transform: Option<GroupTransform>,
    rels: &BTreeMap<String, String>,
    package: &mut Package<R>,
    limits: &Limits,
    media: &mut BTreeMap<String, String>,
    media_bytes: &mut usize,
    theme: &PptxTheme,
    z: usize,
) -> Option<ShapeIr> {
    let (id, name) = shape_identity(node, "nvPicPr");
    let frame = read_frame(node.path(&["spPr", "xfrm"])).map(|frame| match transform {
        Some(transform) => transform.apply(frame),
        None => frame,
    });

    let target = node
        .path(&["blipFill", "blip"])
        .and_then(|blip| blip.attr("embed"))
        .and_then(|rid| rels.get(rid))
        .cloned();

    let image = target.and_then(|part| {
        let bytes = package.try_read(&part)?;
        let hash = content_hash(&bytes);
        let media_id = format!("{part}#{hash:016x}");
        if !media.contains_key(&media_id)
            && bytes.len() <= limits.max_image_bytes
            && *media_bytes + bytes.len() <= limits.max_media_bytes
        {
            *media_bytes += bytes.len();
            media.insert(media_id.clone(), data_url(&part, &bytes));
        }
        Some(ImageIr {
            media_id,
            content_hash: hash,
        })
    });

    Some(ShapeIr {
        id,
        name,
        kind: ShapeKind::Picture,
        placeholder: read_placeholder(node),
        frame,
        z,
        appearance_signature: shape_appearance_signature(node, theme),
        paragraphs: Vec::new(),
        table: None,
        image,
    })
}

fn data_url(part: &str, bytes: &[u8]) -> String {
    let mime = match part
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
    {
        Some(ext) if ext == "png" => "image/png",
        Some(ext) if ext == "jpg" || ext == "jpeg" => "image/jpeg",
        Some(ext) if ext == "gif" => "image/gif",
        Some(ext) if ext == "svg" => "image/svg+xml",
        Some(ext) if ext == "bmp" => "image/bmp",
        Some(ext) if ext == "webp" => "image/webp",
        _ => "application/octet-stream",
    };
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn parse_graphic_frame(
    node: &Node,
    transform: Option<GroupTransform>,
    related_paragraphs: Vec<ParagraphIr>,
    theme: &PptxTheme,
    z: usize,
) -> Option<ShapeIr> {
    let (id, name) = shape_identity(node, "nvGraphicFramePr");
    let frame = read_frame(node.child("xfrm")).map(|frame| match transform {
        Some(transform) => transform.apply(frame),
        None => frame,
    });

    let table = node
        .path(&["graphic", "graphicData", "tbl"])
        .map(read_table);
    let mut paragraphs = if table.is_none() {
        read_math_paragraphs(node)
    } else {
        Vec::new()
    };
    if paragraphs.is_empty() {
        paragraphs = related_paragraphs;
    }
    let kind = if table.is_some() {
        ShapeKind::Table
    } else if paragraphs.is_empty() {
        ShapeKind::Graphic
    } else {
        // 数式・チャート・SmartArtはgraphicFrameだが、構造付きテキストとして
        // 描画・比較できる。
        ShapeKind::Text
    };

    Some(ShapeIr {
        id,
        name,
        kind,
        placeholder: read_placeholder(node),
        frame,
        z,
        appearance_signature: shape_appearance_signature(node, theme),
        paragraphs,
        table,
        image: None,
    })
}

const MAX_CHART_POINTS: usize = 10_000;

fn plain_paragraph(text: String) -> ParagraphIr {
    ParagraphIr {
        text,
        level: 0,
        bold: false,
        size: None,
        align: None,
        format_spans: Vec::new(),
    }
}

/// graphicFrame の relationship からチャートXMLを読み、タイトルと系列キャッシュ
/// を比較可能なテキストへ落とす。埋め込みワークブックを開かなくても、通常の
/// PowerPointが保持する表示用キャッシュで値の変更を検出できる。
fn read_chart_paragraphs<R: Read + Seek>(
    frame: &Node,
    rels: &BTreeMap<String, String>,
    package: &mut Package<R>,
) -> Vec<ParagraphIr> {
    let chart_part = frame
        .descendants()
        .into_iter()
        .find(|node| node.name == "chart")
        .and_then(|chart| chart.attr("r:id"))
        .and_then(|rid| rels.get(rid));
    let Some(chart) = chart_part.and_then(|part| package.try_xml(part)) else {
        return Vec::new();
    };
    chart_paragraphs(&chart)
}

fn chart_paragraphs(chart: &Node) -> Vec<ParagraphIr> {
    fn descendant_text(node: &Node, names: &[&str]) -> String {
        node.descendants()
            .into_iter()
            .filter(|child| names.contains(&child.name.as_str()))
            .map(Node::text_content)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn points(node: Option<&Node>) -> BTreeMap<usize, String> {
        let Some(node) = node else {
            return BTreeMap::new();
        };
        node.descendants()
            .into_iter()
            .filter(|child| child.name == "pt")
            .filter_map(|point| {
                let index = point.attr("idx")?.parse().ok()?;
                let value = point.child("v")?.text_content();
                Some((index, value))
            })
            .take(MAX_CHART_POINTS)
            .collect()
    }

    let mut out = Vec::new();
    if let Some(title) = chart
        .descendants()
        .into_iter()
        .find(|node| node.name == "title")
    {
        let text = descendant_text(title, &["t", "v"]);
        if !text.is_empty() {
            out.push(plain_paragraph(text));
        }
    }

    for (series_index, series) in chart
        .descendants()
        .into_iter()
        .filter(|node| node.name == "ser")
        .enumerate()
    {
        let name = series
            .child("tx")
            .map(|node| descendant_text(node, &["t", "v"]))
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| format!("Series {}", series_index + 1));
        let categories = points(series.child("cat").or_else(|| series.child("xVal")));
        let values = points(series.child("val").or_else(|| series.child("yVal")));
        let mut indices: Vec<usize> = categories.keys().chain(values.keys()).copied().collect();
        indices.sort_unstable();
        indices.dedup();
        indices.truncate(MAX_CHART_POINTS);

        let details = if indices.is_empty() {
            // キャッシュを持たないチャートでも、参照範囲の変更は見えるようにする。
            descendant_text(series, &["f"])
        } else {
            indices
                .into_iter()
                .map(|index| {
                    let category = categories
                        .get(&index)
                        .cloned()
                        .unwrap_or_else(|| (index + 1).to_string());
                    let value = values.get(&index).cloned().unwrap_or_default();
                    format!("{category}={value}")
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push(plain_paragraph(if details.is_empty() {
            name
        } else {
            format!("{name}: {details}")
        }));
    }
    out
}

const MAX_DIAGRAM_PARAGRAPHS: usize = 10_000;

/// SmartArt の graphicFrame から data model relationship (`r:dm`) を解決し、
/// 各ノードの本文を比較可能な段落へ落とす。表示用Drawingではなくdata modelを
/// 読むことで、レイアウト変更に左右されず論理順と箇条書き階層を維持する。
fn read_diagram_paragraphs<R: Read + Seek>(
    frame: &Node,
    rels: &BTreeMap<String, String>,
    package: &mut Package<R>,
    theme: &PptxTheme,
) -> Vec<ParagraphIr> {
    let diagram_part = frame
        .descendants()
        .into_iter()
        .find(|node| node.name == "relIds")
        .and_then(|relationships| relationships.attr("r:dm"))
        .and_then(|rid| rels.get(rid));
    let Some(diagram) = diagram_part.and_then(|part| package.try_xml(part)) else {
        return Vec::new();
    };
    let Some(points) = diagram.child("ptLst") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for point in points.children_named("pt") {
        let Some(body) = point.child("t") else {
            continue;
        };
        for paragraph in read_paragraphs_with_theme(body, Some(theme)) {
            if !paragraph.text.trim().is_empty() {
                out.push(paragraph);
                if out.len() == MAX_DIAGRAM_PARAGRAPHS {
                    return out;
                }
            }
        }
    }
    out
}

/// graphicFrame に入った Office Math を、比較と表示に使える構造付きテキストへ
/// 落とす。PowerPoint の数式は通常の txBody を持たないため、ここで拾わないと
/// 同じ位置・同じ図形 ID の数式変更が差分なしになってしまう。
fn read_math_paragraphs(node: &Node) -> Vec<ParagraphIr> {
    fn walk(node: &Node, out: &mut Vec<ParagraphIr>) {
        match node.name.as_str() {
            "oMathPara" => {
                let text = node
                    .children_named("oMath")
                    .map(render_math)
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                if !text.is_empty() {
                    out.push(plain_paragraph(text));
                }
            }
            "oMath" => {
                let text = render_math(node);
                if !text.is_empty() {
                    out.push(plain_paragraph(text));
                }
            }
            _ => {
                for child in &node.children {
                    walk(child, out);
                }
            }
        }
    }

    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

fn render_math(node: &Node) -> String {
    fn child(node: &Node, name: &str) -> String {
        node.child(name).map(render_math).unwrap_or_default()
    }

    fn property_char(node: &Node, property: &str, default: &str) -> String {
        node.path(&[property, "chr"])
            .and_then(|chr| chr.attr("val"))
            .unwrap_or(default)
            .to_string()
    }

    match node.name.as_str() {
        "t" => node.text.clone(),
        "f" => format!("({})/({})", child(node, "num"), child(node, "den")),
        "sSup" => format!("{}^({})", child(node, "e"), child(node, "sup")),
        "sSub" => format!("{}_({})", child(node, "e"), child(node, "sub")),
        "sSubSup" => format!(
            "{}_({})^({})",
            child(node, "e"),
            child(node, "sub"),
            child(node, "sup")
        ),
        "rad" => {
            let degree = child(node, "deg");
            let expression = child(node, "e");
            if degree.is_empty() {
                format!("√({expression})")
            } else {
                format!("√[{degree}]({expression})")
            }
        }
        "nary" => {
            let operator = property_char(node, "naryPr", "∑");
            let subscript = child(node, "sub");
            let superscript = child(node, "sup");
            let expression = child(node, "e");
            format!("{operator}_({subscript})^({superscript})({expression})")
        }
        "acc" => {
            let accent = property_char(node, "accPr", "ˆ");
            format!("{accent}({})", child(node, "e"))
        }
        "bar" => {
            let position = node
                .path(&["barPr", "pos"])
                .and_then(|value| value.attr("val"))
                .unwrap_or("top");
            if position == "bot" {
                format!("_bar({})", child(node, "e"))
            } else {
                format!("¯({})", child(node, "e"))
            }
        }
        "groupChr" => {
            let character = property_char(node, "groupChrPr", "⏞");
            format!("{character}({})", child(node, "e"))
        }
        "d" => {
            let open = node
                .path(&["dPr", "begChr"])
                .and_then(|value| value.attr("val"))
                .unwrap_or("(");
            let close = node
                .path(&["dPr", "endChr"])
                .and_then(|value| value.attr("val"))
                .unwrap_or(")");
            format!("{open}{}{close}", child(node, "e"))
        }
        "func" => format!("{}({})", child(node, "fName"), child(node, "e")),
        "limLow" => format!("{}_[{}]", child(node, "e"), child(node, "lim")),
        "limUpp" => format!("{}^[{}]", child(node, "e"), child(node, "lim")),
        "m" => {
            let rows = node
                .children_named("mr")
                .map(render_math)
                .collect::<Vec<_>>()
                .join("; ");
            format!("[{rows}]")
        }
        "mr" => node
            .children_named("e")
            .map(render_math)
            .collect::<Vec<_>>()
            .join(", "),
        "eqArr" => node
            .children_named("e")
            .map(render_math)
            .collect::<Vec<_>>()
            .join("; "),
        "box" | "borderBox" => format!("[{}]", child(node, "e")),
        name if name.ends_with("Pr") || matches!(name, "ctrlPr" | "rPr") => String::new(),
        _ => {
            let mut text = node.text.clone();
            for child in &node.children {
                text.push_str(&render_math(child));
            }
            text
        }
    }
}

fn read_table(tbl: &Node) -> TableIr {
    let column_widths = tbl
        .child("tblGrid")
        .map(|grid| {
            grid.children_named("gridCol")
                .map(|col| col.attr_i64("w").unwrap_or(0))
                .collect()
        })
        .unwrap_or_default();

    let mut rows = Vec::new();
    let mut row_heights = Vec::new();
    for tr in tbl.children_named("tr") {
        row_heights.push(tr.attr_i64("h").unwrap_or(0));
        let cells = tr
            .children_named("tc")
            .map(|tc| {
                tc.child("txBody")
                    .map(|body| {
                        read_paragraphs(body)
                            .into_iter()
                            .map(|p| p.text)
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default()
            })
            .collect();
        rows.push(cells);
    }

    TableIr {
        rows,
        column_widths,
        row_heights,
    }
}

fn read_paragraphs(body: &Node) -> Vec<ParagraphIr> {
    read_paragraphs_with_theme(body, None)
}

fn read_paragraphs_with_theme(body: &Node, theme: Option<&PptxTheme>) -> Vec<ParagraphIr> {
    body.children_named("p")
        .map(|paragraph| {
            let properties = paragraph.child("pPr");
            let default_run_properties = properties.and_then(|p| p.child("defRPr"));
            let mut text = String::new();
            let mut bold = false;
            let mut size = None;
            let mut first_run = true;
            let mut format_spans: Vec<FormatSpanIr> = Vec::new();
            for child in &paragraph.children {
                match child.name.as_str() {
                    // a:r (通常の run) と a:fld (ページ番号などの自動フィールド)
                    "r" | "fld" => {
                        let run_text = child.child("t").map(Node::text_content).unwrap_or_default();
                        if let Some(t) = child.child("t") {
                            text.push_str(&t.text_content());
                        }
                        append_format_span(
                            &mut format_spans,
                            run_text.chars().count(),
                            default_run_properties,
                            child.child("rPr"),
                            theme,
                        );
                        if first_run {
                            let direct = child.child("rPr");
                            bold =
                                direct
                                    .and_then(|properties| properties.attr("b"))
                                    .or_else(|| {
                                        default_run_properties
                                            .and_then(|properties| properties.attr("b"))
                                    })
                                    == Some("1");
                            size = direct
                                .and_then(|properties| properties.attr_i64("sz"))
                                .or_else(|| {
                                    default_run_properties
                                        .and_then(|properties| properties.attr_i64("sz"))
                                })
                                .map(|value| value as f32 / 100.0);
                            first_run = false;
                        }
                    }
                    // PowerPointの版によっては数式がgraphicFrameではなく、
                    // 段落内のa14:m / m:oMathとして入る。
                    "m" | "oMath" | "oMathPara" => {
                        for math in read_math_paragraphs(child) {
                            text.push_str(&math.text);
                        }
                    }
                    "br" => {
                        text.push(' ');
                        append_format_span(
                            &mut format_spans,
                            1,
                            default_run_properties,
                            child.child("rPr"),
                            theme,
                        );
                    }
                    _ => {}
                }
            }
            let trimmed_text = text.trim_end();
            let removed_characters = text.chars().count() - trimmed_text.chars().count();
            trim_format_spans_end(&mut format_spans, removed_characters);
            ParagraphIr {
                text: trimmed_text.to_string(),
                level: properties
                    .and_then(|p| p.attr_i64("lvl"))
                    .unwrap_or(0)
                    .clamp(0, 8) as u8,
                bold,
                size,
                align: properties
                    .and_then(|p| p.attr("algn"))
                    .map(|value| value.to_string()),
                format_spans,
            }
        })
        .collect()
}

fn trim_format_spans_end(spans: &mut Vec<FormatSpanIr>, mut characters: usize) {
    while characters > 0 {
        let Some(last) = spans.last_mut() else {
            break;
        };
        if last.characters > characters {
            last.characters -= characters;
            break;
        }
        characters -= last.characters;
        spans.pop();
    }
}

fn append_format_span(
    spans: &mut Vec<FormatSpanIr>,
    characters: usize,
    default_r_pr: Option<&Node>,
    r_pr: Option<&Node>,
    theme: Option<&PptxTheme>,
) {
    if characters == 0 {
        return;
    }
    let style = run_style_signature(default_r_pr, r_pr, theme);
    match spans.last_mut() {
        Some(previous) if previous.style == style => previous.characters += characters,
        _ => spans.push(FormatSpanIr { characters, style }),
    }
}

fn run_style_signature(
    default: Option<&Node>,
    direct: Option<&Node>,
    theme: Option<&PptxTheme>,
) -> String {
    const VISUAL_ATTRIBUTES: &[&str] = &[
        "kumimoji",
        "sz",
        "b",
        "i",
        "u",
        "strike",
        "kern",
        "cap",
        "spc",
        "normalizeH",
        "baseline",
    ];
    const VISUAL_CHILDREN: &[&str] = &[
        "ln",
        "noFill",
        "solidFill",
        "gradFill",
        "blipFill",
        "pattFill",
        "grpFill",
        "effectLst",
        "effectDag",
        "highlight",
        "latin",
        "ea",
        "cs",
        "sym",
    ];

    let mut attrs = BTreeMap::new();
    let mut children = BTreeMap::new();
    for properties in [default, direct].into_iter().flatten() {
        for (name, value) in &properties.attrs {
            if VISUAL_ATTRIBUTES.contains(&name.as_str()) {
                let value = if matches!(name.as_str(), "sz" | "kern" | "spc" | "baseline") {
                    canonical_xml_integer(value).unwrap_or_else(|| value.clone())
                } else {
                    value.clone()
                };
                attrs.insert(name.clone(), value);
            }
        }
        for child in &properties.children {
            if !VISUAL_CHILDREN.contains(&child.name.as_str()) {
                continue;
            }
            let group = match child.name.as_str() {
                "noFill" | "solidFill" | "gradFill" | "blipFill" | "pattFill" | "grpFill" => "fill",
                "effectLst" | "effectDag" => "effect",
                other => other,
            };
            children.insert(group.to_string(), node_signature_with_theme(child, theme));
        }
    }
    attrs.retain(|name, value| {
        !matches!(
            (name.as_str(), value.as_str()),
            ("b" | "i" | "kumimoji" | "normalizeH", "0" | "false")
                | ("u", "none")
                | ("strike", "noStrike")
                | ("cap", "none")
                | ("baseline" | "spc", "0")
        )
    });
    if attrs.is_empty() && children.is_empty() {
        return String::new();
    }

    let mut signature = String::new();
    for (name, value) in attrs {
        signature.push('|');
        signature.push_str(&name);
        signature.push('=');
        signature.push_str(&value);
    }
    for child in children.into_values() {
        signature.push('{');
        signature.push_str(&child);
        signature.push('}');
    }
    signature
}

fn canonical_xml_integer(value: &str) -> Option<String> {
    value.parse::<i64>().ok().map(|value| value.to_string())
}

fn node_signature(node: &Node) -> String {
    node_signature_with_theme(node, None)
}

fn color_signature_rgb(signature: &str) -> Option<&str> {
    let value = color_signature_attribute(signature, "val")
        .or_else(|| color_signature_attribute(signature, "lastClr"))?;
    (value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(value)
}

fn color_signature_attribute<'a>(signature: &'a str, name: &str) -> Option<&'a str> {
    signature.split('|').find_map(|attribute| {
        let (attribute_name, value) = attribute.split_once('=')?;
        (attribute_name == name).then_some(value)
    })
}

fn drawingml_signature_opacity(signature: &str) -> Option<u32> {
    let value = signature
        .strip_suffix('}')?
        .rsplit_once("{alpha|val=")?
        .1
        .parse::<u32>()
        .ok()?;
    (value <= 100_000).then_some(value)
}

fn drawingml_rendered_signature_rgb(signature: &str) -> Option<&str> {
    let base_signature = signature
        .split_once('{')
        .map_or(signature, |(base, _)| base);
    let rgb = color_signature_rgb(base_signature)?;
    let base = format!("srgbClr|val={rgb}");
    if signature == base {
        return Some(rgb);
    }
    let opacity = drawingml_signature_opacity(signature)?;
    (signature == format!("{base}{{alpha|val={opacity}}}")).then_some(rgb)
}

#[derive(Clone, Copy)]
enum DrawingmlColorTransform {
    Tint(f64),
    Shade(f64),
    Complement,
    Inverse,
    Gray,
    Gamma,
    InverseGamma,
    Channel(usize, f64),
    ChannelModulation(usize, f64),
    ChannelOffset(usize, f64),
    Hue(f64),
    HueModulation(f64),
    HueOffset(f64),
    Saturation(f64),
    SaturationModulation(f64),
    SaturationOffset(f64),
    Luminance(f64),
    LuminanceModulation(f64),
    LuminanceOffset(f64),
}

fn drawingml_color_transform(node: &Node) -> Option<DrawingmlColorTransform> {
    match node.name.as_str() {
        "comp" => return Some(DrawingmlColorTransform::Complement),
        "inv" => return Some(DrawingmlColorTransform::Inverse),
        "gray" => return Some(DrawingmlColorTransform::Gray),
        "gamma" => return Some(DrawingmlColorTransform::Gamma),
        "invGamma" => return Some(DrawingmlColorTransform::InverseGamma),
        _ => {}
    }
    let value = node.attr("val")?;
    match node.name.as_str() {
        "tint" | "shade" => {
            let amount = value.parse::<u32>().ok()?;
            (amount <= 100_000).then(|| {
                let amount = f64::from(amount) / 100_000.0;
                if node.name == "tint" {
                    DrawingmlColorTransform::Tint(amount)
                } else {
                    DrawingmlColorTransform::Shade(amount)
                }
            })
        }
        "red" | "green" | "blue" => value.parse::<i32>().ok().map(|amount| {
            DrawingmlColorTransform::Channel(
                match node.name.as_str() {
                    "red" => 0,
                    "green" => 1,
                    _ => 2,
                },
                f64::from(amount) / 100_000.0,
            )
        }),
        "redMod" | "greenMod" | "blueMod" => value.parse::<u32>().ok().map(|amount| {
            DrawingmlColorTransform::ChannelModulation(
                match node.name.as_str() {
                    "redMod" => 0,
                    "greenMod" => 1,
                    _ => 2,
                },
                f64::from(amount) / 100_000.0,
            )
        }),
        "redOff" | "greenOff" | "blueOff" => value.parse::<i32>().ok().map(|amount| {
            DrawingmlColorTransform::ChannelOffset(
                match node.name.as_str() {
                    "redOff" => 0,
                    "greenOff" => 1,
                    _ => 2,
                },
                f64::from(amount) / 100_000.0,
            )
        }),
        "hue" => {
            let amount = value.parse::<u32>().ok()?;
            (amount <= 21_600_000)
                .then(|| DrawingmlColorTransform::Hue(f64::from(amount) / 21_600_000.0))
        }
        "sat" => value
            .parse::<i32>()
            .ok()
            .map(|amount| DrawingmlColorTransform::Saturation(f64::from(amount) / 100_000.0)),
        "lum" => value
            .parse::<i32>()
            .ok()
            .map(|amount| DrawingmlColorTransform::Luminance(f64::from(amount) / 100_000.0)),
        "lumMod" => value.parse::<u32>().ok().map(|amount| {
            DrawingmlColorTransform::LuminanceModulation(f64::from(amount) / 100_000.0)
        }),
        "lumOff" => value
            .parse::<i32>()
            .ok()
            .map(|amount| DrawingmlColorTransform::LuminanceOffset(f64::from(amount) / 100_000.0)),
        "hueMod" => value
            .parse::<u32>()
            .ok()
            .map(|amount| DrawingmlColorTransform::HueModulation(f64::from(amount) / 100_000.0)),
        "hueOff" => value
            .parse::<i32>()
            .ok()
            .map(|amount| DrawingmlColorTransform::HueOffset(f64::from(amount) / 21_600_000.0)),
        "satMod" => value.parse::<u32>().ok().map(|amount| {
            DrawingmlColorTransform::SaturationModulation(f64::from(amount) / 100_000.0)
        }),
        "satOff" => value
            .parse::<i32>()
            .ok()
            .map(|amount| DrawingmlColorTransform::SaturationOffset(f64::from(amount) / 100_000.0)),
        _ => None,
    }
}

fn drawingml_rgb_to_hsl(channels: [f64; 3]) -> (f64, f64, f64) {
    let [red, green, blue] = channels;
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    let delta = maximum - minimum;
    let luminance = (maximum + minimum) / 2.0;
    if delta.abs() < f64::EPSILON {
        return (0.0, 0.0, luminance);
    }
    let saturation = if luminance > 0.5 {
        delta / (2.0 - maximum - minimum)
    } else {
        delta / (maximum + minimum)
    };
    let hue = if (maximum - red).abs() < f64::EPSILON {
        (green - blue) / delta + if green < blue { 6.0 } else { 0.0 }
    } else if (maximum - green).abs() < f64::EPSILON {
        (blue - red) / delta + 2.0
    } else {
        (red - green) / delta + 4.0
    } / 6.0;
    (hue, saturation, luminance)
}

fn drawingml_hsl_to_rgb(hue: f64, saturation: f64, luminance: f64) -> [f64; 3] {
    if saturation.abs() < f64::EPSILON {
        return [luminance; 3];
    }
    let high = if luminance < 0.5 {
        luminance * (1.0 + saturation)
    } else {
        luminance + saturation - luminance * saturation
    };
    let low = 2.0 * luminance - high;
    let hue_to_rgb = |mut offset: f64| {
        if offset < 0.0 {
            offset += 1.0;
        }
        if offset > 1.0 {
            offset -= 1.0;
        }
        if offset < 1.0 / 6.0 {
            low + (high - low) * 6.0 * offset
        } else if offset < 0.5 {
            high
        } else if offset < 2.0 / 3.0 {
            low + (high - low) * (2.0 / 3.0 - offset) * 6.0
        } else {
            low
        }
    };
    [
        hue_to_rgb(hue + 1.0 / 3.0),
        hue_to_rgb(hue),
        hue_to_rgb(hue - 1.0 / 3.0),
    ]
}

fn drawingml_hsl_rgb(hue: &str, saturation: &str, luminance: &str) -> Option<String> {
    let hue = hue.parse::<u32>().ok()?;
    let saturation = saturation.parse::<u32>().ok()?;
    let luminance = luminance.parse::<u32>().ok()?;
    if hue > 21_600_000 || saturation > 100_000 || luminance > 100_000 {
        return None;
    }
    let channels = drawingml_hsl_to_rgb(
        (f64::from(hue) / 21_600_000.0).rem_euclid(1.0),
        f64::from(saturation) / 100_000.0,
        f64::from(luminance) / 100_000.0,
    );
    Some(format!(
        "{:02X}{:02X}{:02X}",
        (channels[0] * 255.0 + 1e-9).round() as u8,
        (channels[1] * 255.0 + 1e-9).round() as u8,
        (channels[2] * 255.0 + 1e-9).round() as u8,
    ))
}

fn drawingml_hsl_node_rgb(node: &Node) -> Option<String> {
    (node.name == "hslClr")
        .then(|| drawingml_hsl_rgb(node.attr("hue")?, node.attr("sat")?, node.attr("lum")?))
        .flatten()
}

fn drawingml_hsl_signature_rgb(signature: &str) -> Option<String> {
    if !signature.starts_with("hslClr|") || signature.contains('{') {
        return None;
    }
    drawingml_hsl_rgb(
        color_signature_attribute(signature, "hue")?,
        color_signature_attribute(signature, "sat")?,
        color_signature_attribute(signature, "lum")?,
    )
}

fn drawingml_scrgb_rgb(red: &str, green: &str, blue: &str) -> Option<String> {
    let channels = [
        red.parse::<u32>().ok()?,
        green.parse::<u32>().ok()?,
        blue.parse::<u32>().ok()?,
    ];
    if channels.iter().any(|channel| *channel > 100_000) {
        return None;
    }
    let channels = channels.map(|channel| drawingml_srgb_gamma(f64::from(channel) / 100_000.0));
    Some(format!(
        "{:02X}{:02X}{:02X}",
        (channels[0] * 255.0 + 1e-9).round() as u8,
        (channels[1] * 255.0 + 1e-9).round() as u8,
        (channels[2] * 255.0 + 1e-9).round() as u8,
    ))
}

fn drawingml_scrgb_node_rgb(node: &Node) -> Option<String> {
    (node.name == "scrgbClr")
        .then(|| drawingml_scrgb_rgb(node.attr("r")?, node.attr("g")?, node.attr("b")?))
        .flatten()
}

fn drawingml_scrgb_signature_rgb(signature: &str) -> Option<String> {
    if !signature.starts_with("scrgbClr|") || signature.contains('{') {
        return None;
    }
    drawingml_scrgb_rgb(
        color_signature_attribute(signature, "r")?,
        color_signature_attribute(signature, "g")?,
        color_signature_attribute(signature, "b")?,
    )
}

fn drawingml_preset_rgb(value: &str) -> Option<&'static str> {
    Some(match value {
        "aliceBlue" => "F0F8FF",
        "antiqueWhite" => "FAEBD7",
        "aqua" | "cyan" => "00FFFF",
        "aquamarine" => "7FFFD4",
        "azure" => "F0FFFF",
        "beige" => "F5F5DC",
        "bisque" => "FFE4C4",
        "black" => "000000",
        "blanchedAlmond" => "FFEBCD",
        "blue" => "0000FF",
        "blueViolet" => "8A2BE2",
        "brown" => "A52A2A",
        "burlyWood" => "DEB887",
        "cadetBlue" => "5F9EA0",
        "chartreuse" => "7FFF00",
        "chocolate" => "D2691E",
        "coral" => "FF7F50",
        "cornflowerBlue" => "6495ED",
        "cornsilk" => "FFF8DC",
        "crimson" => "DC143C",
        "dkBlue" => "00008B",
        "dkCyan" => "008B8B",
        "dkGoldenrod" => "B8860B",
        "dkGray" => "A9A9A9",
        "dkGreen" => "006400",
        "dkKhaki" => "BDB76B",
        "dkMagenta" => "8B008B",
        "dkOliveGreen" => "556B2F",
        "dkOrange" => "FF8C00",
        "dkOrchid" => "9932CC",
        "dkRed" => "8B0000",
        "dkSalmon" => "E9967A",
        "dkSeaGreen" => "8FBC8B",
        "dkSlateBlue" => "483D8B",
        "dkSlateGray" => "2F4F4F",
        "dkTurquoise" => "00CED1",
        "dkViolet" => "9400D3",
        "deepPink" => "FF1493",
        "deepSkyBlue" => "00BFFF",
        "dimGray" => "696969",
        "dodgerBlue" => "1E90FF",
        "firebrick" => "B22222",
        "floralWhite" => "FFFAF0",
        "forestGreen" => "228B22",
        "fuchsia" | "magenta" => "FF00FF",
        "gainsboro" => "DCDCDC",
        "ghostWhite" => "F8F8FF",
        "gold" => "FFD700",
        "goldenrod" => "DAA520",
        "gray" => "808080",
        "green" => "008000",
        "greenYellow" => "ADFF2F",
        "honeydew" => "F0FFF0",
        "hotPink" => "FF69B4",
        "indianRed" => "CD5C5C",
        "indigo" => "4B0082",
        "ivory" => "FFFFF0",
        "khaki" => "F0E68C",
        "lavender" => "E6E6FA",
        "lavenderBlush" => "FFF0F5",
        "lawnGreen" => "7CFC00",
        "lemonChiffon" => "FFFACD",
        "ltBlue" => "ADD8E6",
        "ltCoral" => "F08080",
        "ltCyan" => "E0FFFF",
        "ltGoldenrodYellow" => "FAFA78",
        "ltGray" => "D3D3D3",
        "ltGreen" => "90EE90",
        "ltPink" => "FFB6C1",
        "ltSalmon" => "FFA07A",
        "ltSeaGreen" => "20B2AA",
        "ltSkyBlue" => "87CEFA",
        "ltSlateGray" => "778899",
        "ltSteelBlue" => "B0C4DE",
        "ltYellow" => "FFFFE0",
        "lime" => "00FF00",
        "limeGreen" => "32CD32",
        "linen" => "FAF0E6",
        "maroon" => "800000",
        "medAquamarine" => "66CDAA",
        "medBlue" => "0000CD",
        "medOrchid" => "BA55D3",
        "medPurple" => "9370DB",
        "medSeaGreen" => "3CB371",
        "medSlateBlue" => "7B68EE",
        "medSpringGreen" => "00FA9A",
        "medTurquoise" => "48D1CC",
        "medVioletRed" => "C71585",
        "midnightBlue" => "191970",
        "mintCream" => "F5FFFA",
        "mistyRose" => "FFE4E1",
        "moccasin" => "FFE4B5",
        "navajoWhite" => "FFDEAD",
        "navy" => "000080",
        "oldLace" => "FDF5E6",
        "olive" => "808000",
        "oliveDrab" => "6B8E23",
        "orange" => "FFA500",
        "orangeRed" => "FF4500",
        "orchid" => "DA70D6",
        "paleGoldenrod" => "EEE8AA",
        "paleGreen" => "98FB98",
        "paleTurquoise" => "AFEEEE",
        "paleVioletRed" => "DB7093",
        "papayaWhip" => "FFEFD5",
        "peachPuff" => "FFDAB9",
        "peru" => "CD853F",
        "pink" => "FFC0CB",
        "plum" => "DDA0DD",
        "powderBlue" => "B0E0E6",
        "purple" => "800080",
        "red" => "FF0000",
        "rosyBrown" => "BC8F8F",
        "royalBlue" => "4169E1",
        "saddleBrown" => "8B4513",
        "salmon" => "FA8072",
        "sandyBrown" => "F4A460",
        "seaGreen" => "2E8B57",
        "seaShell" => "FFF5EE",
        "sienna" => "A0522D",
        "silver" => "C0C0C0",
        "skyBlue" => "87CEEB",
        "slateBlue" => "6A5ACD",
        "slateGray" => "708090",
        "snow" => "FFFAFA",
        "springGreen" => "00FF7F",
        "steelBlue" => "4682B4",
        "tan" => "D2B48C",
        "teal" => "008080",
        "thistle" => "D8BFD8",
        "tomato" => "FF6347",
        "turquoise" => "40E0D0",
        "violet" => "EE82EE",
        "wheat" => "F5DEB3",
        "white" => "FFFFFF",
        "whiteSmoke" => "F5F5F5",
        "yellow" => "FFFF00",
        "yellowGreen" => "9ACD32",
        _ => return None,
    })
}

fn drawingml_preset_node_rgb(node: &Node) -> Option<&'static str> {
    (node.name == "prstClr")
        .then(|| drawingml_preset_rgb(node.attr("val")?))
        .flatten()
}

fn drawingml_srgb_gamma(channel: f64) -> f64 {
    if channel > 0.003_130_8 {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    } else {
        12.92 * channel
    }
}

fn drawingml_srgb_inverse_gamma(channel: f64) -> f64 {
    if channel <= 0.040_45 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

fn drawingml_srgb_gray(channels: [f64; 3]) -> f64 {
    let linear = channels.map(drawingml_srgb_inverse_gamma);
    let luminance = linear[0] * (87_098.0 / 409_605.0)
        + linear[1] * (175_762.0 / 245_763.0)
        + linear[2] * (12_673.0 / 175_545.0);
    drawingml_srgb_gamma(luminance)
}

fn apply_drawingml_color_transforms(base: &str, transforms: &[Node]) -> Option<String> {
    if base.len() != 6 || !base.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut channels = [
        f64::from(u8::from_str_radix(&base[0..2], 16).ok()?) / 255.0,
        f64::from(u8::from_str_radix(&base[2..4], 16).ok()?) / 255.0,
        f64::from(u8::from_str_radix(&base[4..6], 16).ok()?) / 255.0,
    ];
    let mut applied = false;
    for transform in transforms {
        let Some(transform) = drawingml_color_transform(transform) else {
            continue;
        };
        applied = true;
        match transform {
            DrawingmlColorTransform::Tint(amount) => {
                for channel in &mut channels {
                    *channel = *channel * amount + 1.0 - amount;
                }
            }
            DrawingmlColorTransform::Shade(amount) => {
                for channel in &mut channels {
                    *channel *= amount;
                }
            }
            DrawingmlColorTransform::Complement => {
                let sum = channels.iter().copied().fold(f64::INFINITY, f64::min)
                    + channels.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                for channel in &mut channels {
                    *channel = sum - *channel;
                }
            }
            DrawingmlColorTransform::Inverse => {
                for channel in &mut channels {
                    *channel = 1.0 - *channel;
                }
            }
            DrawingmlColorTransform::Gray => {
                channels = [drawingml_srgb_gray(channels); 3];
            }
            DrawingmlColorTransform::Gamma => {
                channels = channels.map(drawingml_srgb_gamma);
            }
            DrawingmlColorTransform::InverseGamma => {
                channels = channels.map(drawingml_srgb_inverse_gamma);
            }
            DrawingmlColorTransform::Channel(channel, amount) => {
                channels[channel] = amount.clamp(0.0, 1.0);
            }
            DrawingmlColorTransform::ChannelModulation(channel, amount) => {
                channels[channel] = (channels[channel] * amount).clamp(0.0, 1.0);
            }
            DrawingmlColorTransform::ChannelOffset(channel, amount) => {
                channels[channel] = (channels[channel] + amount).clamp(0.0, 1.0);
            }
            DrawingmlColorTransform::Hue(amount) => {
                let (_, saturation, luminance) = drawingml_rgb_to_hsl(channels);
                channels = drawingml_hsl_to_rgb(amount.rem_euclid(1.0), saturation, luminance);
            }
            DrawingmlColorTransform::HueModulation(amount) => {
                let (hue, saturation, luminance) = drawingml_rgb_to_hsl(channels);
                channels =
                    drawingml_hsl_to_rgb((hue * amount).rem_euclid(1.0), saturation, luminance);
            }
            DrawingmlColorTransform::HueOffset(amount) => {
                let (hue, saturation, luminance) = drawingml_rgb_to_hsl(channels);
                channels =
                    drawingml_hsl_to_rgb((hue + amount).rem_euclid(1.0), saturation, luminance);
            }
            DrawingmlColorTransform::Saturation(amount) => {
                let (hue, _, luminance) = drawingml_rgb_to_hsl(channels);
                channels = drawingml_hsl_to_rgb(hue, amount.clamp(0.0, 1.0), luminance);
            }
            DrawingmlColorTransform::SaturationModulation(amount) => {
                let (hue, saturation, luminance) = drawingml_rgb_to_hsl(channels);
                channels =
                    drawingml_hsl_to_rgb(hue, (saturation * amount).clamp(0.0, 1.0), luminance);
            }
            DrawingmlColorTransform::SaturationOffset(amount) => {
                let (hue, saturation, luminance) = drawingml_rgb_to_hsl(channels);
                channels =
                    drawingml_hsl_to_rgb(hue, (saturation + amount).clamp(0.0, 1.0), luminance);
            }
            DrawingmlColorTransform::Luminance(amount) => {
                let (hue, saturation, _) = drawingml_rgb_to_hsl(channels);
                channels = drawingml_hsl_to_rgb(hue, saturation, amount.clamp(0.0, 1.0));
            }
            DrawingmlColorTransform::LuminanceModulation(amount) => {
                let (hue, saturation, luminance) = drawingml_rgb_to_hsl(channels);
                channels =
                    drawingml_hsl_to_rgb(hue, saturation, (luminance * amount).clamp(0.0, 1.0));
            }
            DrawingmlColorTransform::LuminanceOffset(amount) => {
                let (hue, saturation, luminance) = drawingml_rgb_to_hsl(channels);
                channels =
                    drawingml_hsl_to_rgb(hue, saturation, (luminance + amount).clamp(0.0, 1.0));
            }
        }
    }
    applied.then(|| {
        format!(
            "{:02X}{:02X}{:02X}",
            (channels[0] * 255.0 + 1e-9).round().clamp(0.0, 255.0) as u8,
            (channels[1] * 255.0 + 1e-9).round().clamp(0.0, 255.0) as u8,
            (channels[2] * 255.0 + 1e-9).round().clamp(0.0, 255.0) as u8,
        )
    })
}

#[derive(Clone, Copy)]
enum DrawingmlAlphaTransform {
    Alpha(f64),
    Modulation(f64),
    Offset(f64),
}

fn drawingml_alpha_transform(node: &Node) -> Option<DrawingmlAlphaTransform> {
    let value = node.attr("val")?;
    match node.name.as_str() {
        "alpha" => {
            let amount = value.parse::<u32>().ok()?;
            (amount <= 100_000)
                .then(|| DrawingmlAlphaTransform::Alpha(f64::from(amount) / 100_000.0))
        }
        "alphaMod" => value
            .parse::<u32>()
            .ok()
            .map(|amount| DrawingmlAlphaTransform::Modulation(f64::from(amount) / 100_000.0)),
        "alphaOff" => value
            .parse::<i32>()
            .ok()
            .map(|amount| DrawingmlAlphaTransform::Offset(f64::from(amount) / 100_000.0)),
        _ => None,
    }
}

fn apply_drawingml_alpha_transforms(transforms: &[Node], initial_opacity: u32) -> Option<u32> {
    let mut opacity = f64::from(initial_opacity) / 100_000.0;
    let mut applied = false;
    for transform in transforms {
        let Some(transform) = drawingml_alpha_transform(transform) else {
            continue;
        };
        applied = true;
        opacity = match transform {
            DrawingmlAlphaTransform::Alpha(amount) => amount,
            DrawingmlAlphaTransform::Modulation(amount) => opacity * amount,
            DrawingmlAlphaTransform::Offset(amount) => opacity + amount,
        }
        .clamp(0.0, 1.0);
    }
    applied.then(|| (opacity * 100_000.0 + 1e-9).round() as u32)
}

fn node_signature_with_theme(node: &Node, theme: Option<&PptxTheme>) -> String {
    let mut attrs = node.attrs.clone();
    for (name, value) in &mut attrs {
        let name = name.rsplit(':').next().unwrap_or(name);
        if ((node.name == "srgbClr" && name == "val")
            || (node.name == "sysClr" && name == "lastClr"))
            && value.len() == 6
            && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            *value = value.to_ascii_uppercase();
        }
    }
    let resolved_color = (node.name == "schemeClr")
        .then(|| node.attr("val"))
        .flatten()
        .and_then(|value| theme.and_then(|theme| theme.resolve(value)));
    let resolved_rendered_rgb = resolved_color.and_then(drawingml_rendered_signature_rgb);
    let resolved_opacity = resolved_color
        .filter(|_| resolved_rendered_rgb.is_some())
        .and_then(drawingml_signature_opacity);
    let system_rgb = (node.name == "sysClr")
        .then(|| node.attr("lastClr"))
        .flatten()
        .filter(|value| value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let resolved_system_rgb = resolved_color
        .filter(|signature| signature.starts_with("sysClr|") && !signature.contains('{'))
        .and_then(color_signature_rgb);
    let resolved_hsl_rgb = resolved_color.and_then(drawingml_hsl_signature_rgb);
    let direct_hsl_rgb = drawingml_hsl_node_rgb(node);
    let resolved_scrgb_rgb = resolved_color.and_then(drawingml_scrgb_signature_rgb);
    let direct_scrgb_rgb = drawingml_scrgb_node_rgb(node);
    let direct_preset_rgb = drawingml_preset_node_rgb(node);
    let base_rgb = resolved_rendered_rgb
        .or(resolved_hsl_rgb.as_deref())
        .or(resolved_scrgb_rgb.as_deref())
        .or(system_rgb)
        .or(direct_hsl_rgb.as_deref())
        .or(direct_scrgb_rgb.as_deref())
        .or(direct_preset_rgb)
        .or_else(|| (node.name == "srgbClr").then(|| node.attr("val")).flatten());
    let transformed_color =
        base_rgb.and_then(|color| apply_drawingml_color_transforms(color, &node.children));
    let rendered_color = transformed_color.or_else(|| {
        (resolved_rendered_rgb.is_some()
            || resolved_system_rgb.is_some()
            || resolved_hsl_rgb.is_some()
            || resolved_scrgb_rgb.is_some()
            || system_rgb.is_some()
            || direct_hsl_rgb.is_some()
            || direct_scrgb_rgb.is_some()
            || direct_preset_rgb.is_some())
        .then(|| base_rgb.map(str::to_ascii_uppercase))
        .flatten()
    });
    let transformed_alpha =
        apply_drawingml_alpha_transforms(&node.children, resolved_opacity.unwrap_or(100_000));
    let rendered_alpha = transformed_alpha.or(resolved_opacity);
    if resolved_color.is_some() || rendered_color.is_some() {
        attrs.retain(|(name, _)| {
            let name = name.rsplit(':').next().unwrap_or(name);
            name != "val"
                && !(name == "lastClr" && rendered_color.is_some())
                && !(node.name == "hslClr"
                    && rendered_color.is_some()
                    && matches!(name, "hue" | "sat" | "lum"))
                && !(node.name == "scrgbClr"
                    && rendered_color.is_some()
                    && matches!(name, "r" | "g" | "b"))
        });
    }
    attrs.sort();
    let mut signature = rendered_color.as_ref().map_or_else(
        || resolved_color.map_or_else(|| node.name.clone(), str::to_string),
        |color| format!("srgbClr|val={color}"),
    );
    for (name, value) in attrs {
        signature.push('|');
        signature.push_str(&name);
        signature.push('=');
        signature.push_str(&value);
    }
    if let Some(opacity) = rendered_alpha.filter(|opacity| *opacity < 100_000) {
        signature.push_str("{alpha|val=");
        signature.push_str(&opacity.to_string());
        signature.push('}');
    }
    for child in &node.children {
        if rendered_color.is_some() && drawingml_color_transform(child).is_some() {
            continue;
        }
        if transformed_alpha.is_some() && drawingml_alpha_transform(child).is_some() {
            continue;
        }
        signature.push('{');
        signature.push_str(&node_signature_with_theme(child, theme));
        signature.push('}');
    }
    if !node.text.is_empty() {
        signature.push('[');
        signature.push_str(&node.text);
        signature.push(']');
    }
    signature
}

/// 図形の外観だけを、XML の属性順に依存しない形へ正規化する。
///
/// xfrm の off/ext は Frame と許容誤差つきで比較するため除き、回転・反転だけを
/// ここへ含める。これにより Office が保存時に座標を丸めても二重に差分にならない。
fn shape_appearance_signature(node: &Node, theme: &PptxTheme) -> String {
    const VISUAL_PROPERTIES: &[&str] = &[
        "prstGeom",
        "custGeom",
        "noFill",
        "solidFill",
        "gradFill",
        "blipFill",
        "pattFill",
        "grpFill",
        "ln",
        "effectLst",
        "effectDag",
        "scene3d",
        "sp3d",
    ];

    let properties = node.child("spPr");
    let transform = properties
        .and_then(|properties| properties.child("xfrm"))
        .or_else(|| node.child("xfrm"));
    let mut parts = Vec::new();

    if let Some(transform) = transform {
        let mut attrs: Vec<(String, String)> = transform
            .attrs
            .iter()
            .filter_map(|(name, value)| {
                if !matches!(name.as_str(), "rot" | "flipH" | "flipV") {
                    return None;
                }
                let value = if name == "rot" {
                    canonical_xml_integer(value).unwrap_or_else(|| value.clone())
                } else {
                    value.clone()
                };
                (!matches!(value.as_str(), "0" | "false")).then(|| (name.clone(), value))
            })
            .collect();
        attrs.sort();
        for (name, value) in attrs {
            parts.push(format!("xfrm|{name}={value}"));
        }
    }

    if let Some(properties) = properties {
        parts.extend(
            properties
                .children
                .iter()
                .filter(|child| VISUAL_PROPERTIES.contains(&child.name.as_str()))
                .map(|child| node_signature_with_theme(child, Some(theme))),
        );
    }
    if let Some(style) = node.child("style") {
        parts.push(node_signature_with_theme(style, Some(theme)));
    }
    if let Some(table) = node.path(&["graphic", "graphicData", "tbl"]) {
        parts.extend(table_appearance_signature(table, theme));
    }
    parts.join("{")
}

fn table_appearance_signature(table: &Node, theme: &PptxTheme) -> Vec<String> {
    const TABLE_VISUAL_CHILDREN: &[&str] = &[
        "tableStyleId",
        "noFill",
        "solidFill",
        "gradFill",
        "blipFill",
        "pattFill",
        "grpFill",
        "effectLst",
        "effectDag",
    ];
    const CELL_VISUAL_CHILDREN: &[&str] = &[
        "lnL",
        "lnR",
        "lnT",
        "lnB",
        "lnTlToBr",
        "lnBlToTr",
        "cell3D",
        "noFill",
        "solidFill",
        "gradFill",
        "blipFill",
        "pattFill",
        "grpFill",
    ];

    let mut out = Vec::new();
    if let Some(properties) = table.child("tblPr") {
        let mut attrs: Vec<(String, String)> = properties
            .attrs
            .iter()
            .filter(|(_, value)| !matches!(value.as_str(), "0" | "false"))
            .cloned()
            .collect();
        attrs.sort();
        for (name, value) in attrs {
            out.push(format!("tblPr|{name}={value}"));
        }
        out.extend(
            properties
                .children
                .iter()
                .filter(|child| TABLE_VISUAL_CHILDREN.contains(&child.name.as_str()))
                .map(|child| node_signature_with_theme(child, Some(theme))),
        );
    }

    for (row_index, row) in table.children_named("tr").enumerate() {
        for (cell_index, cell) in row.children_named("tc").enumerate() {
            if let Some(properties) = cell.child("tcPr") {
                let mut attrs: Vec<(String, String)> = properties
                    .attrs
                    .iter()
                    .filter_map(|(name, value)| {
                        let value = if matches!(name.as_str(), "marL" | "marR" | "marT" | "marB") {
                            canonical_xml_integer(value).unwrap_or_else(|| value.clone())
                        } else {
                            value.clone()
                        };
                        (!matches!(
                            (name.as_str(), value.as_str()),
                            ("marL" | "marR", "91440")
                                | ("marT" | "marB", "45720")
                                | ("vert", "horz")
                                | ("anchor", "t")
                                | ("anchorCtr", "0" | "false")
                        ))
                        .then(|| (name.clone(), value))
                    })
                    .collect();
                attrs.sort();
                for (name, value) in attrs {
                    out.push(format!("tc:{row_index}:{cell_index}|{name}={value}"));
                }
                out.extend(
                    properties
                        .children
                        .iter()
                        .filter(|child| CELL_VISUAL_CHILDREN.contains(&child.name.as_str()))
                        .map(|child| {
                            format!(
                                "tc:{row_index}:{cell_index}|{}",
                                node_signature_with_theme(child, Some(theme))
                            )
                        }),
                );
            }
            if let Some(body) = cell.child("txBody") {
                for (paragraph_index, paragraph) in read_paragraphs_with_theme(body, Some(theme))
                    .iter()
                    .enumerate()
                {
                    for (span_index, span) in paragraph.format_spans.iter().enumerate() {
                        if !span.style.is_empty() {
                            out.push(format!(
                                "tc:{row_index}:{cell_index}:p:{paragraph_index}:r:{span_index}|{}|{}",
                                span.characters, span.style
                            ));
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use proptest::prelude::*;

    use super::*;

    const PRESET_COLOR_CASES: &[(&str, &str)] = &[
        ("aliceBlue", "F0F8FF"),
        ("aqua", "00FFFF"),
        ("black", "000000"),
        ("blueViolet", "8A2BE2"),
        ("dkGoldenrod", "B8860B"),
        ("dkSeaGreen", "8FBC8B"),
        ("deepPink", "FF1493"),
        ("gray", "808080"),
        ("greenYellow", "ADFF2F"),
        ("ltSteelBlue", "B0C4DE"),
        ("medVioletRed", "C71585"),
        ("navy", "000080"),
        ("orangeRed", "FF4500"),
        ("paleTurquoise", "AFEEEE"),
        ("red", "FF0000"),
        ("whiteSmoke", "F5F5F5"),
        ("yellowGreen", "9ACD32"),
    ];

    fn equation_shape(base: &str, exponent: u16) -> ShapeIr {
        let xml = format!(
            r#"<p:graphicFrame xmlns:p="p" xmlns:a="a" xmlns:m="m">
                <p:nvGraphicFramePr><p:cNvPr id="7" name="Equation"/></p:nvGraphicFramePr>
                <a:graphic><a:graphicData>
                    <m:oMathPara><m:oMath><m:sSup>
                        <m:e><m:r><m:t>{base}</m:t></m:r></m:e>
                        <m:sup><m:r><m:t>{exponent}</m:t></m:r></m:sup>
                    </m:sSup></m:oMath></m:oMathPara>
                </a:graphicData></a:graphic>
            </p:graphicFrame>"#
        );
        let node = xml::parse(xml.as_bytes()).unwrap();
        parse_graphic_frame(&node, None, Vec::new(), &PptxTheme::default(), 0).unwrap()
    }

    fn rendered_math(fragment: &str) -> String {
        let xml = format!(r#"<m:oMath xmlns:m="m">{fragment}</m:oMath>"#);
        render_math(&xml::parse(xml.as_bytes()).unwrap())
    }

    fn presentation(shape: ShapeIr) -> PresentationIr {
        PresentationIr {
            width: 10_000,
            height: 10_000,
            slides: vec![SlideIr {
                part: "ppt/slides/slide1.xml".to_string(),
                index: 1,
                shapes: vec![shape],
                notes: None,
            }],
            media: BTreeMap::new(),
        }
    }

    fn text_shape(transform_attributes: &str, appearance: &str) -> ShapeIr {
        let xml = format!(
            r#"<p:sp xmlns:p="p" xmlns:a="a">
                <p:nvSpPr><p:cNvPr id="1" name="Text"/></p:nvSpPr>
                <p:spPr>
                    <a:xfrm {transform_attributes}><a:off x="0" y="0"/><a:ext cx="1000" cy="1000"/></a:xfrm>
                    <a:prstGeom prst="rect"/>{appearance}
                </p:spPr>
                <p:txBody><a:p><a:r><a:t>同じ本文</a:t></a:r></a:p></p:txBody>
            </p:sp>"#
        );
        parse_sp(
            &xml::parse(xml.as_bytes()).unwrap(),
            None,
            &[],
            &PptxTheme::default(),
            0,
        )
        .unwrap()
    }

    fn test_theme(accent1: &str, accent2: &str) -> PptxTheme {
        let root = xml::parse(
            format!(
                r#"<a:theme xmlns:a="a"><a:themeElements><a:clrScheme name="Office"><a:accent1><a:srgbClr val="{accent1}"/></a:accent1><a:accent2><a:srgbClr val="{accent2}"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#
            )
            .as_bytes(),
        )
        .unwrap();
        let mut theme = PptxTheme::default();
        theme.merge_theme(&root);
        theme
    }

    fn rendered_hsl_rgb(hue: u32, saturation: u32, luminance: u32) -> String {
        let hue = f64::from(hue) / 21_600_000.0;
        let saturation = f64::from(saturation) / 100_000.0;
        let luminance = f64::from(luminance) / 100_000.0;
        let chroma = (1.0 - (2.0 * luminance - 1.0).abs()) * saturation;
        let sector = hue * 6.0;
        let secondary = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
        let [red, green, blue] = match sector as usize {
            0 => [chroma, secondary, 0.0],
            1 => [secondary, chroma, 0.0],
            2 => [0.0, chroma, secondary],
            3 => [0.0, secondary, chroma],
            4 => [secondary, 0.0, chroma],
            _ => [chroma, 0.0, secondary],
        };
        let minimum = luminance - chroma / 2.0;
        let render = |channel: f64| (channel + minimum) * 255.0;
        format!(
            "{:02X}{:02X}{:02X}",
            (render(red) + 1e-9).round() as u8,
            (render(green) + 1e-9).round() as u8,
            (render(blue) + 1e-9).round() as u8,
        )
    }

    fn rendered_scrgb_rgb(red: u32, green: u32, blue: u32) -> String {
        let render = |channel: u32| {
            let linear = f64::from(channel) / 100_000.0;
            let encoded = if linear > 0.0031308 {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            } else {
                12.92 * linear
            };
            (encoded * 255.0 + 1e-9).round() as u8
        };
        format!(
            "{:02X}{:02X}{:02X}",
            render(red),
            render(green),
            render(blue),
        )
    }

    fn rendered_tint_or_shade(rgb: &str, amount: u32, tint: bool) -> String {
        let factor = f64::from(amount) / 100_000.0;
        let render = |offset: usize| {
            let channel = f64::from(u8::from_str_radix(&rgb[offset..offset + 2], 16).unwrap());
            let rendered = if tint {
                channel * factor + 255.0 * (1.0 - factor)
            } else {
                channel * factor
            };
            (rendered + 1e-9).round() as u8
        };
        format!("{:02X}{:02X}{:02X}", render(0), render(2), render(4))
    }

    fn themed_text_shape(theme: &PptxTheme, color: &str) -> ShapeIr {
        let root = xml::parse(
            format!(
                r#"<p:sp xmlns:p="p" xmlns:a="a"><p:nvSpPr><p:cNvPr id="1" name="Text"/></p:nvSpPr><p:spPr/><p:txBody><a:p><a:r><a:rPr><a:solidFill>{color}</a:solidFill></a:rPr><a:t>同じ本文</a:t></a:r></a:p></p:txBody></p:sp>"#
            )
            .as_bytes(),
        )
        .unwrap();
        parse_sp(&root, None, &[], theme, 0).unwrap()
    }

    #[test]
    fn detects_shape_appearance_changes_when_text_and_frame_are_unchanged() {
        for (old_transform, old_appearance, new_transform, new_appearance) in [
            (
                "",
                r#"<a:solidFill><a:srgbClr val="FF0000"/></a:solidFill>"#,
                "",
                r#"<a:solidFill><a:srgbClr val="0000FF"/></a:solidFill>"#,
            ),
            (
                "",
                r#"<a:ln w="12700"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln>"#,
                "",
                r#"<a:ln w="25400"><a:solidFill><a:srgbClr val="000000"/></a:solidFill></a:ln>"#,
            ),
            ("rot=\"0\"", "", "rot=\"60000\"", ""),
        ] {
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(text_shape(old_transform, old_appearance)),
                &presentation(text_shape(new_transform, new_appearance)),
                &crate::diff::DiffOptions::default(),
            );

            assert_eq!(diff.summary.shape_appearances_changed, 1, "{diff:?}");
            assert!(diff.slides[0].shapes[0].appearance_changed, "{diff:?}");
        }
    }

    #[test]
    fn detects_referenced_theme_color_changes() {
        let old =
            load_presentation(&themed_shape_pptx("4F81BD", "C0504D"), &Limits::default()).unwrap();
        let new =
            load_presentation(&themed_shape_pptx("AA0000", "C0504D"), &Limits::default()).unwrap();

        let diff = crate::diff::diff_presentations(
            "old.pptx",
            "new.pptx",
            &old,
            &new,
            &crate::diff::DiffOptions::default(),
        );

        assert_eq!(diff.summary.shape_appearances_changed, 1, "{diff:?}");
        assert!(diff.slides[0].shapes[0].appearance_changed, "{diff:?}");
    }

    #[test]
    fn ignores_unreferenced_theme_color_changes() {
        let old =
            load_presentation(&themed_shape_pptx("4F81BD", "C0504D"), &Limits::default()).unwrap();
        let new =
            load_presentation(&themed_shape_pptx("4F81BD", "00AA00"), &Limits::default()).unwrap();

        let diff = crate::diff::diff_presentations(
            "old.pptx",
            "new.pptx",
            &old,
            &new,
            &crate::diff::DiffOptions::default(),
        );

        assert!(!diff.has_changes(), "{diff:?}");
    }

    #[test]
    fn detects_referenced_theme_text_color_changes() {
        let diff = crate::diff::diff_presentations(
            "old.pptx",
            "new.pptx",
            &presentation(themed_text_shape(
                &test_theme("FF0000", "00FF00"),
                r#"<a:schemeClr val="accent1"/>"#,
            )),
            &presentation(themed_text_shape(
                &test_theme("0000FF", "00FF00"),
                r#"<a:schemeClr val="accent1"/>"#,
            )),
            &crate::diff::DiffOptions::default(),
        );

        assert_eq!(diff.summary.paragraphs_changed, 1, "{diff:?}");
        assert!(diff.slides[0].shapes[0].paragraphs[0].format_changed);
    }

    #[test]
    fn ignores_unreferenced_theme_text_color_changes() {
        let diff = crate::diff::diff_presentations(
            "old.pptx",
            "new.pptx",
            &presentation(themed_text_shape(
                &test_theme("FF0000", "00FF00"),
                r#"<a:schemeClr val="accent1"/>"#,
            )),
            &presentation(themed_text_shape(
                &test_theme("FF0000", "0000FF"),
                r#"<a:schemeClr val="accent1"/>"#,
            )),
            &crate::diff::DiffOptions::default(),
        );

        assert!(!diff.has_changes(), "{diff:?}");
    }

    #[test]
    fn equivalent_theme_and_direct_text_colors_are_not_changes() {
        let theme = test_theme("FF0000", "00FF00");
        let diff = crate::diff::diff_presentations(
            "old.pptx",
            "new.pptx",
            &presentation(themed_text_shape(&theme, r#"<a:schemeClr val="accent1"/>"#)),
            &presentation(themed_text_shape(&theme, r#"<a:srgbClr val="FF0000"/>"#)),
            &crate::diff::DiffOptions::default(),
        );

        assert!(!diff.has_changes(), "{diff:?}");
    }

    #[test]
    fn explicit_default_transform_is_not_an_appearance_change() {
        let diff = crate::diff::diff_presentations(
            "old.pptx",
            "new.pptx",
            &presentation(text_shape("", "")),
            &presentation(text_shape("rot=\"0\" flipH=\"false\" flipV=\"0\"", "")),
            &crate::diff::DiffOptions::default(),
        );

        assert!(!diff.has_changes(), "{diff:?}");
    }

    fn table_shape(fill: &str) -> ShapeIr {
        let xml = format!(
            r#"<p:graphicFrame xmlns:p="p" xmlns:a="a">
                <p:nvGraphicFramePr><p:cNvPr id="2" name="Table"/></p:nvGraphicFramePr>
                <p:xfrm><a:off x="0" y="0"/><a:ext cx="1000" cy="1000"/></p:xfrm>
                <a:graphic><a:graphicData><a:tbl>
                    <a:tblGrid><a:gridCol w="1000"/></a:tblGrid>
                    <a:tr h="500"><a:tc>
                        <a:txBody><a:p><a:r><a:t>同じ値</a:t></a:r></a:p></a:txBody>
                        <a:tcPr><a:solidFill><a:srgbClr val="{fill}"/></a:solidFill></a:tcPr>
                    </a:tc></a:tr>
                </a:tbl></a:graphicData></a:graphic>
            </p:graphicFrame>"#
        );
        parse_graphic_frame(
            &xml::parse(xml.as_bytes()).unwrap(),
            None,
            Vec::new(),
            &PptxTheme::default(),
            0,
        )
        .unwrap()
    }

    #[test]
    fn detects_table_cell_appearance_changes_when_values_are_unchanged() {
        let diff = crate::diff::diff_presentations(
            "old.pptx",
            "new.pptx",
            &presentation(table_shape("FF0000")),
            &presentation(table_shape("0000FF")),
            &crate::diff::DiffOptions::default(),
        );

        assert_eq!(diff.summary.shape_appearances_changed, 1, "{diff:?}");
    }

    #[test]
    fn detects_table_cell_text_format_changes_when_values_are_unchanged() {
        fn shape(text_color: &str) -> ShapeIr {
            let xml = format!(
                r#"<p:graphicFrame xmlns:p="p" xmlns:a="a">
                    <p:nvGraphicFramePr><p:cNvPr id="2" name="Table"/></p:nvGraphicFramePr>
                    <p:xfrm><a:off x="0" y="0"/><a:ext cx="1000" cy="1000"/></p:xfrm>
                    <a:graphic><a:graphicData><a:tbl>
                        <a:tblGrid><a:gridCol w="1000"/></a:tblGrid>
                        <a:tr h="500"><a:tc>
                            <a:txBody><a:p><a:r><a:rPr><a:solidFill><a:srgbClr val="{text_color}"/></a:solidFill></a:rPr><a:t>同じ値</a:t></a:r></a:p></a:txBody>
                            <a:tcPr/>
                        </a:tc></a:tr>
                    </a:tbl></a:graphicData></a:graphic>
                </p:graphicFrame>"#
            );
            parse_graphic_frame(
                &xml::parse(xml.as_bytes()).unwrap(),
                None,
                Vec::new(),
                &PptxTheme::default(),
                0,
            )
            .unwrap()
        }

        let diff = crate::diff::diff_presentations(
            "old.pptx",
            "new.pptx",
            &presentation(shape("FF0000")),
            &presentation(shape("0000FF")),
            &crate::diff::DiffOptions::default(),
        );

        assert_eq!(diff.summary.table_cells_changed, 0, "{diff:?}");
        assert_eq!(diff.summary.shape_appearances_changed, 1, "{diff:?}");
    }

    #[test]
    fn detects_format_changes_after_the_first_run_in_a_paragraph() {
        fn shape(body: &str) -> ShapeIr {
            let body = xml::parse(body.as_bytes()).unwrap();
            ShapeIr {
                id: "1".to_string(),
                name: "Text".to_string(),
                kind: ShapeKind::Text,
                placeholder: None,
                frame: None,
                z: 0,
                appearance_signature: String::new(),
                paragraphs: read_paragraphs(&body),
                table: None,
                image: None,
            }
        }

        let old = presentation(shape(
            r#"<a:txBody xmlns:a="a"><a:p>
                <a:r><a:rPr b="0"/><a:t>売上</a:t></a:r>
                <a:r><a:rPr b="0"/><a:t>100万円</a:t></a:r>
            </a:p></a:txBody>"#,
        ));
        let new = presentation(shape(
            r#"<a:txBody xmlns:a="a"><a:p>
                <a:r><a:rPr b="0"/><a:t>売上</a:t></a:r>
                <a:r><a:rPr b="1"/><a:t>100万円</a:t></a:r>
            </a:p></a:txBody>"#,
        ));

        let diff = crate::diff::diff_presentations(
            "old.pptx",
            "new.pptx",
            &old,
            &new,
            &crate::diff::DiffOptions::default(),
        );
        assert_eq!(diff.summary.paragraphs_changed, 1, "{diff:?}");
    }

    #[test]
    fn detects_default_run_format_changes() {
        fn shape(default_color: &str) -> ShapeIr {
            let body = xml::parse(
                format!(
                    r#"<a:txBody xmlns:a="a"><a:p><a:pPr><a:defRPr><a:solidFill><a:srgbClr val="{default_color}"/></a:solidFill></a:defRPr></a:pPr><a:r><a:t>同じ本文</a:t></a:r></a:p></a:txBody>"#
                )
                .as_bytes(),
            )
            .unwrap();
            ShapeIr {
                id: "1".to_string(),
                name: "Text".to_string(),
                kind: ShapeKind::Text,
                placeholder: None,
                frame: None,
                z: 0,
                appearance_signature: String::new(),
                paragraphs: read_paragraphs(&body),
                table: None,
                image: None,
            }
        }

        let diff = crate::diff::diff_presentations(
            "old.pptx",
            "new.pptx",
            &presentation(shape("FF0000")),
            &presentation(shape("0000FF")),
            &crate::diff::DiffOptions::default(),
        );

        assert_eq!(diff.summary.paragraphs_changed, 1, "{diff:?}");
        assert!(diff.slides[0].shapes[0].paragraphs[0].format_changed);
    }

    #[test]
    fn equivalent_default_format_and_run_splitting_are_not_changes() {
        fn paragraphs(body: &str) -> Vec<ParagraphIr> {
            read_paragraphs(&xml::parse(body.as_bytes()).unwrap())
        }

        let old = paragraphs(
            r#"<a:txBody xmlns:a="a"><a:p>
                <a:r><a:t>売上100万円</a:t></a:r>
            </a:p></a:txBody>"#,
        );
        let new = paragraphs(
            r#"<a:txBody xmlns:a="a"><a:p>
                <a:r><a:rPr b="0" dirty="0"/><a:t>売上</a:t></a:r>
                <a:r><a:rPr b="0"/><a:t>100万円</a:t></a:r>
            </a:p></a:txBody>"#,
        );

        let changes = crate::diff::diff_presentations(
            "old.pptx",
            "new.pptx",
            &presentation(ShapeIr {
                id: "1".to_string(),
                name: "Text".to_string(),
                kind: ShapeKind::Text,
                placeholder: None,
                frame: None,
                z: 0,
                appearance_signature: String::new(),
                paragraphs: old,
                table: None,
                image: None,
            }),
            &presentation(ShapeIr {
                id: "1".to_string(),
                name: "Text".to_string(),
                kind: ShapeKind::Text,
                placeholder: None,
                frame: None,
                z: 0,
                appearance_signature: String::new(),
                paragraphs: new,
                table: None,
                image: None,
            }),
            &crate::diff::DiffOptions::default(),
        );
        assert_eq!(changes.summary.paragraphs_changed, 0, "{changes:?}");
    }

    fn themed_shape_pptx(accent1: &str, accent2: &str) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        let parts = [
            (
                "ppt/presentation.xml",
                r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldSz cx="10000" cy="10000"/><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst></p:presentation>"#.to_string(),
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                r#"<Relationships><Relationship Id="rId1" Target="slides/slide1.xml"/></Relationships>"#.to_string(),
            ),
            (
                "ppt/slides/slide1.xml",
                r#"<p:sld xmlns:p="p" xmlns:a="a"><p:cSld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="1" name="Theme shape"/></p:nvSpPr><p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1000" cy="1000"/></a:xfrm><a:prstGeom prst="rect"/><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></p:spPr><p:txBody><a:p><a:r><a:t>same</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#.to_string(),
            ),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                r#"<Relationships><Relationship Id="rIdLayout" Target="../slideLayouts/slideLayout1.xml"/></Relationships>"#.to_string(),
            ),
            (
                "ppt/slideLayouts/slideLayout1.xml",
                r#"<p:sldLayout xmlns:p="p"><p:cSld><p:spTree/></p:cSld></p:sldLayout>"#.to_string(),
            ),
            (
                "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
                r#"<Relationships><Relationship Id="rIdMaster" Target="../slideMasters/slideMaster1.xml"/></Relationships>"#.to_string(),
            ),
            (
                "ppt/slideMasters/slideMaster1.xml",
                r#"<p:sldMaster xmlns:p="p"><p:cSld><p:spTree/></p:cSld></p:sldMaster>"#.to_string(),
            ),
            (
                "ppt/slideMasters/_rels/slideMaster1.xml.rels",
                r#"<Relationships><Relationship Id="rIdTheme" Target="../theme/theme1.xml"/></Relationships>"#.to_string(),
            ),
            (
                "ppt/theme/theme1.xml",
                format!(
                    r#"<a:theme xmlns:a="a"><a:themeElements><a:clrScheme name="Office"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:accent1><a:srgbClr val="{accent1}"/></a:accent1><a:accent2><a:srgbClr val="{accent2}"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#
                ),
            ),
        ];
        for (name, content) in parts {
            zip.start_file(name, options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn chart_pptx(value: u16) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        let parts = [
            (
                "ppt/presentation.xml",
                r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldSz cx="10000" cy="10000"/><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst></p:presentation>"#.to_string(),
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                r#"<Relationships><Relationship Id="rId1" Target="slides/slide1.xml"/></Relationships>"#.to_string(),
            ),
            (
                "ppt/slides/slide1.xml",
                r#"<p:sld xmlns:p="p" xmlns:a="a" xmlns:c="c" xmlns:r="r"><p:cSld><p:spTree><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="7" name="Chart"/></p:nvGraphicFramePr><p:xfrm><a:off x="0" y="0"/><a:ext cx="1000" cy="1000"/></p:xfrm><a:graphic><a:graphicData><c:chart r:id="rIdChart"/></a:graphicData></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>"#.to_string(),
            ),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                r#"<Relationships><Relationship Id="rIdChart" Target="../charts/chart1.xml"/></Relationships>"#.to_string(),
            ),
            (
                "ppt/charts/chart1.xml",
                format!(
                    r#"<c:chartSpace xmlns:c="c" xmlns:a="a"><c:chart><c:title><c:tx><c:rich><a:p><a:r><a:t>売上</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:barChart><c:ser><c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>商品</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:strCache><c:pt idx="0"><c:v>Q1</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>{value}</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#
                ),
            ),
        ];
        for (name, content) in parts {
            zip.start_file(name, options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn smartart_pptx(value: u16) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        let parts = [
            (
                "ppt/presentation.xml",
                r#"<p:presentation xmlns:p="p" xmlns:r="r"><p:sldSz cx="10000" cy="10000"/><p:sldIdLst><p:sldId id="256" r:id="rId1"/></p:sldIdLst></p:presentation>"#.to_string(),
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                r#"<Relationships><Relationship Id="rId1" Target="slides/slide1.xml"/></Relationships>"#.to_string(),
            ),
            (
                "ppt/slides/slide1.xml",
                r#"<p:sld xmlns:p="p" xmlns:a="a" xmlns:dgm="dgm" xmlns:r="r"><p:cSld><p:spTree><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="8" name="SmartArt"/></p:nvGraphicFramePr><p:xfrm><a:off x="0" y="0"/><a:ext cx="1000" cy="1000"/></p:xfrm><a:graphic><a:graphicData><dgm:relIds r:dm="rIdDiagram"/></a:graphicData></a:graphic></p:graphicFrame></p:spTree></p:cSld></p:sld>"#.to_string(),
            ),
            (
                "ppt/slides/_rels/slide1.xml.rels",
                r#"<Relationships><Relationship Id="rIdDiagram" Target="../diagrams/data1.xml"/></Relationships>"#.to_string(),
            ),
            (
                "ppt/diagrams/data1.xml",
                format!(
                    r#"<dgm:dataModel xmlns:dgm="dgm" xmlns:a="a"><dgm:ptLst><dgm:pt modelId="root" type="doc"><dgm:t><a:p/></dgm:t></dgm:pt><dgm:pt modelId="plan"><dgm:t><a:p><a:r><a:t>計画</a:t></a:r></a:p></dgm:t></dgm:pt><dgm:pt modelId="progress"><dgm:t><a:p><a:pPr lvl="1"/><a:r><a:t>進捗 {value}%</a:t></a:r></a:p></dgm:t></dgm:pt></dgm:ptLst></dgm:dataModel>"#
                ),
            ),
        ];
        for (name, content) in parts {
            zip.start_file(name, options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn resolves_relative_targets() {
        assert_eq!(
            resolve_target("ppt/slides", "../media/image1.png"),
            "ppt/media/image1.png"
        );
        assert_eq!(
            resolve_target("ppt", "slides/slide1.xml"),
            "ppt/slides/slide1.xml"
        );
        assert_eq!(
            resolve_target("", "ppt/presentation.xml"),
            "ppt/presentation.xml"
        );
    }

    #[test]
    fn group_transform_maps_child_space() {
        let transform = GroupTransform {
            off_x: 1000,
            off_y: 2000,
            child_x: 0,
            child_y: 0,
            scale_x: 2.0,
            scale_y: 0.5,
        };
        let moved = transform.apply(Frame {
            x: 10,
            y: 10,
            width: 100,
            height: 100,
        });
        assert_eq!(moved.x, 1020);
        assert_eq!(moved.y, 2005);
        assert_eq!(moved.width, 200);
        assert_eq!(moved.height, 50);
    }

    #[test]
    fn reads_paragraph_text_and_level() {
        let body = xml::parse(
            br#"<txBody><a:p><a:pPr lvl="1" algn="ctr"/><a:r><a:rPr b="1" sz="1800"/><a:t>hello</a:t></a:r><a:r><a:t> world</a:t></a:r></a:p></txBody>"#,
        )
        .unwrap();
        let paragraphs = read_paragraphs(&body);
        assert_eq!(paragraphs.len(), 1);
        assert_eq!(paragraphs[0].text, "hello world");
        assert_eq!(paragraphs[0].level, 1);
        assert!(paragraphs[0].bold);
        assert_eq!(paragraphs[0].size, Some(18.0));
        assert_eq!(paragraphs[0].align.as_deref(), Some("ctr"));
    }

    #[test]
    fn reads_office_math_superscript_as_structured_text() {
        let shape = equation_shape("x", 2);
        assert_eq!(shape.kind, ShapeKind::Text);
        assert_eq!(shape.paragraphs.len(), 1);
        assert_eq!(shape.paragraphs[0].text, "x^(2)");
    }

    #[test]
    fn preserves_common_office_math_structures() {
        assert_eq!(
            rendered_math(
                "<m:f><m:num><m:r><m:t>1</m:t></m:r></m:num><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f>"
            ),
            "(1)/(2)"
        );
        assert_eq!(
            rendered_math(
                "<m:rad><m:deg><m:r><m:t>3</m:t></m:r></m:deg><m:e><m:r><m:t>x</m:t></m:r></m:e></m:rad>"
            ),
            "√[3](x)"
        );
        assert_eq!(
            rendered_math(
                "<m:m><m:mr><m:e><m:r><m:t>a</m:t></m:r></m:e><m:e><m:r><m:t>b</m:t></m:r></m:e></m:mr><m:mr><m:e><m:r><m:t>c</m:t></m:r></m:e><m:e><m:r><m:t>d</m:t></m:r></m:e></m:mr></m:m>"
            ),
            "[a, b; c, d]"
        );
    }

    #[test]
    fn reads_inline_office_math_fields_and_breaks() {
        let source = r#"<txBody xmlns:a="a" xmlns:a14="a14" xmlns:m="m"><a:p>
                <a:fld><a:t>式:</a:t></a:fld>
                <a14:m><m:oMath><m:sSub><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sub><m:r><m:t>i</m:t></m:r></m:sub></m:sSub></m:oMath></a14:m>
                <a:br/><a:r><a:t>続き</a:t></a:r>
            </a:p></txBody>"#;
        let body = xml::parse(source.as_bytes()).unwrap();
        let paragraphs = read_paragraphs(&body);
        assert_eq!(paragraphs[0].text, "式:x_(i) 続き");
    }

    #[test]
    fn reads_chart_title_series_categories_and_values() {
        let deck = load_presentation(&chart_pptx(120), &Limits::default()).unwrap();
        let shape = &deck.slides[0].shapes[0];
        let texts: Vec<_> = shape
            .paragraphs
            .iter()
            .map(|paragraph| paragraph.text.as_str())
            .collect();
        assert_eq!(texts, vec!["売上", "商品: Q1=120"]);
    }

    #[test]
    fn reads_smartart_labels_and_levels() {
        let deck = load_presentation(&smartart_pptx(42), &Limits::default()).unwrap();
        let shape = &deck.slides[0].shapes[0];
        let paragraphs = &shape.paragraphs;
        assert_eq!(shape.kind, ShapeKind::Text);
        assert_eq!(paragraphs.len(), 2);
        assert_eq!(paragraphs[0].text, "計画");
        assert_eq!(paragraphs[0].level, 0);
        assert_eq!(paragraphs[1].text, "進捗 42%");
        assert_eq!(paragraphs[1].level, 1);
    }

    proptest! {
        #[test]
        fn arbitrary_drawingml_gamma_transforms_equal_rendered_rgb(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            inverse in any::<bool>(),
        ) {
            let transform_channel = |channel: u8| {
                let channel = f64::from(channel) / 255.0;
                let rendered = if inverse {
                    if channel <= 0.04045 {
                        channel / 12.92
                    } else {
                        ((channel + 0.055) / 1.055).powf(2.4)
                    }
                } else if channel > 0.0031308 {
                    1.055 * channel.powf(1.0 / 2.4) - 0.055
                } else {
                    12.92 * channel
                };
                (rendered * 255.0 + 1e-9).round() as u8
            };
            let rendered = [red, green, blue].map(transform_channel);
            let transform = if inverse { "invGamma" } else { "gamma" };
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:{transform}/></a:schemeClr>"#
            );
            let direct_color = format!(
                r#"<a:srgbClr val="{:02X}{:02X}{:02X}"/>"#,
                rendered[0], rendered[1], rendered[2]
            );
            let theme = test_theme(&format!("{red:02X}{green:02X}{blue:02X}"), "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn paired_drawingml_gamma_transforms_are_identity(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            inverse_first in any::<bool>(),
        ) {
            let (first, second) = if inverse_first {
                ("invGamma", "gamma")
            } else {
                ("gamma", "invGamma")
            };
            let transformed_color = format!(
                r#"<a:srgbClr val="{red:02X}{green:02X}{blue:02X}"><a:{first}/><a:{second}/></a:srgbClr>"#
            );
            let direct_color = format!(
                r#"<a:srgbClr val="{red:02X}{green:02X}{blue:02X}"/>"#
            );
            let theme = test_theme("000000", "FFFFFF");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &transformed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_gray_equals_rendered_srgb_luminance(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
        ) {
            let linearize = |channel: u8| {
                let channel = f64::from(channel) / 255.0;
                if channel <= 0.04045 {
                    channel / 12.92
                } else {
                    ((channel + 0.055) / 1.055).powf(2.4)
                }
            };
            let luminance = linearize(red) * (87_098.0 / 409_605.0)
                + linearize(green) * (175_762.0 / 245_763.0)
                + linearize(blue) * (12_673.0 / 175_545.0);
            let encoded = if luminance > 0.0031308 {
                1.055 * luminance.powf(1.0 / 2.4) - 0.055
            } else {
                12.92 * luminance
            };
            let gray = (encoded * 255.0 + 1e-9).round() as u8;
            let themed_color = r#"<a:schemeClr val="accent1"><a:gray/></a:schemeClr>"#;
            let direct_color = format!(r#"<a:srgbClr val="{gray:02X}{gray:02X}{gray:02X}"/>"#);
            let theme = test_theme(&format!("{red:02X}{green:02X}{blue:02X}"), "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn repeated_drawingml_gray_is_idempotent(channel in any::<u8>()) {
            let transformed_color = format!(
                r#"<a:srgbClr val="{channel:02X}{channel:02X}{channel:02X}"><a:gray/><a:gray/></a:srgbClr>"#
            );
            let direct_color = format!(
                r#"<a:srgbClr val="{channel:02X}{channel:02X}{channel:02X}"/>"#
            );
            let theme = test_theme("000000", "FFFFFF");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &transformed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_system_color_equals_last_computed_rgb(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            window_text in any::<bool>(),
        ) {
            let rgb = format!("{red:02X}{green:02X}{blue:02X}");
            let system_color = if window_text { "windowText" } else { "window" };
            let system = format!(
                r#"<a:sysClr val="{system_color}" lastClr="{}"/>"#,
                rgb.to_ascii_lowercase()
            );
            let direct = format!(r#"<a:srgbClr val="{rgb}"/>"#);
            let theme = PptxTheme::default();
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &system)),
                &presentation(themed_text_shape(&theme, &direct)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_theme_system_color_equals_last_computed_rgb(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
        ) {
            let rgb = format!("{red:02X}{green:02X}{blue:02X}");
            let root = xml::parse(
                format!(
                    r#"<a:theme xmlns:a="a"><a:themeElements><a:clrScheme name="Office"><a:accent1><a:sysClr val="windowText" lastClr="{}"/></a:accent1></a:clrScheme></a:themeElements></a:theme>"#,
                    rgb.to_ascii_lowercase()
                )
                .as_bytes(),
            )
            .unwrap();
            let mut theme = PptxTheme::default();
            theme.merge_theme(&root);
            let direct = format!(r#"<a:srgbClr val="{rgb}"/>"#);
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(
                    &theme,
                    r#"<a:schemeClr val="accent1"/>"#,
                )),
                &presentation(themed_text_shape(&theme, &direct)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_hsl_color_equals_rendered_rgb(
            hue in 0u32..21_600_000,
            saturation in 0u32..=100_000,
            luminance in 0u32..=100_000,
        ) {
            let rendered = rendered_hsl_rgb(hue, saturation, luminance);
            let hsl = format!(
                r#"<a:hslClr hue="{hue}" sat="{saturation}" lum="{luminance}"/>"#
            );
            let direct = format!(r#"<a:srgbClr val="{rendered}"/>"#);
            let theme = PptxTheme::default();
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &hsl)),
                &presentation(themed_text_shape(&theme, &direct)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_theme_hsl_color_equals_rendered_rgb(
            hue in 0u32..21_600_000,
            saturation in 0u32..=100_000,
            luminance in 0u32..=100_000,
        ) {
            let rendered = rendered_hsl_rgb(hue, saturation, luminance);
            let root = xml::parse(
                format!(
                    r#"<a:theme xmlns:a="a"><a:themeElements><a:clrScheme name="Office"><a:accent1><a:hslClr hue="{hue}" sat="{saturation}" lum="{luminance}"/></a:accent1></a:clrScheme></a:themeElements></a:theme>"#
                )
                .as_bytes(),
            )
            .unwrap();
            let mut theme = PptxTheme::default();
            theme.merge_theme(&root);
            let direct = format!(r#"<a:srgbClr val="{rendered}"/>"#);
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(
                    &theme,
                    r#"<a:schemeClr val="accent1"/>"#,
                )),
                &presentation(themed_text_shape(&theme, &direct)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_scrgb_color_equals_rendered_rgb(
            red in 0u32..=100_000,
            green in 0u32..=100_000,
            blue in 0u32..=100_000,
        ) {
            let rendered = rendered_scrgb_rgb(red, green, blue);
            let scrgb = format!(r#"<a:scrgbClr r="{red}" g="{green}" b="{blue}"/>"#);
            let direct = format!(r#"<a:srgbClr val="{rendered}"/>"#);
            let theme = PptxTheme::default();
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &scrgb)),
                &presentation(themed_text_shape(&theme, &direct)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_theme_scrgb_color_equals_rendered_rgb(
            red in 0u32..=100_000,
            green in 0u32..=100_000,
            blue in 0u32..=100_000,
        ) {
            let rendered = rendered_scrgb_rgb(red, green, blue);
            let root = xml::parse(
                format!(
                    r#"<a:theme xmlns:a="a"><a:themeElements><a:clrScheme name="Office"><a:accent1><a:scrgbClr r="{red}" g="{green}" b="{blue}"/></a:accent1></a:clrScheme></a:themeElements></a:theme>"#
                )
                .as_bytes(),
            )
            .unwrap();
            let mut theme = PptxTheme::default();
            theme.merge_theme(&root);
            let direct = format!(r#"<a:srgbClr val="{rendered}"/>"#);
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(
                    &theme,
                    r#"<a:schemeClr val="accent1"/>"#,
                )),
                &presentation(themed_text_shape(&theme, &direct)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_theme_hsl_transforms_equal_rendered_rgb(
            hue in 0u32..21_600_000,
            saturation in 0u32..=100_000,
            luminance in 0u32..=100_000,
            amount in 0u32..=100_000,
            tint in any::<bool>(),
        ) {
            let transform = if tint { "tint" } else { "shade" };
            let rendered = rendered_tint_or_shade(
                &rendered_hsl_rgb(hue, saturation, luminance),
                amount,
                tint,
            );
            let root = xml::parse(
                format!(
                    r#"<a:theme xmlns:a="a"><a:themeElements><a:clrScheme name="Office"><a:accent1><a:hslClr hue="{hue}" sat="{saturation}" lum="{luminance}"><a:{transform} val="{amount}"/></a:hslClr></a:accent1></a:clrScheme></a:themeElements></a:theme>"#
                )
                .as_bytes(),
            )
            .unwrap();
            let mut theme = PptxTheme::default();
            theme.merge_theme(&root);
            let direct = format!(r#"<a:srgbClr val="{rendered}"/>"#);
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(
                    &theme,
                    r#"<a:schemeClr val="accent1"/>"#,
                )),
                &presentation(themed_text_shape(&theme, &direct)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_theme_scrgb_transforms_equal_rendered_rgb(
            red in 0u32..=100_000,
            green in 0u32..=100_000,
            blue in 0u32..=100_000,
            amount in 0u32..=100_000,
            tint in any::<bool>(),
        ) {
            let transform = if tint { "tint" } else { "shade" };
            let rendered = rendered_tint_or_shade(
                &rendered_scrgb_rgb(red, green, blue),
                amount,
                tint,
            );
            let root = xml::parse(
                format!(
                    r#"<a:theme xmlns:a="a"><a:themeElements><a:clrScheme name="Office"><a:accent1><a:scrgbClr r="{red}" g="{green}" b="{blue}"><a:{transform} val="{amount}"/></a:scrgbClr></a:accent1></a:clrScheme></a:themeElements></a:theme>"#
                )
                .as_bytes(),
            )
            .unwrap();
            let mut theme = PptxTheme::default();
            theme.merge_theme(&root);
            let direct = format!(r#"<a:srgbClr val="{rendered}"/>"#);
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(
                    &theme,
                    r#"<a:schemeClr val="accent1"/>"#,
                )),
                &presentation(themed_text_shape(&theme, &direct)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_preset_colors_equal_rendered_rgb(
            (preset, rgb) in prop::sample::select(PRESET_COLOR_CASES.to_vec()),
            amount in 0u32..=100_000,
            tint in any::<bool>(),
            through_theme in any::<bool>(),
        ) {
            let transform = if tint { "tint" } else { "shade" };
            let rendered = rendered_tint_or_shade(rgb, amount, tint);
            let preset_color = format!(
                r#"<a:prstClr val="{preset}"><a:{transform} val="{amount}"/></a:prstClr>"#
            );
            let mut theme = PptxTheme::default();
            let tested_color = if through_theme {
                let root = xml::parse(
                    format!(
                        r#"<a:theme xmlns:a="a"><a:themeElements><a:clrScheme name="Office"><a:accent1>{preset_color}</a:accent1></a:clrScheme></a:themeElements></a:theme>"#
                    )
                    .as_bytes(),
                )
                .unwrap();
                theme.merge_theme(&root);
                r#"<a:schemeClr val="accent1"/>"#.to_string()
            } else {
                preset_color
            };
            let direct = format!(r#"<a:srgbClr val="{rendered}"/>"#);
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &tested_color)),
                &presentation(themed_text_shape(&theme, &direct)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn drawingml_absolute_hue_follows_the_six_color_wheel(
            step in 0usize..6,
        ) {
            let expected = ["FF0000", "FFFF00", "00FF00", "00FFFF", "0000FF", "FF00FF"][step];
            let hue = step as u32 * 3_600_000;
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:hue val="{hue}"/></a:schemeClr>"#
            );
            let direct_color = format!(r#"<a:srgbClr val="{expected}"/>"#);
            let theme = test_theme("FF0000", "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_absolute_saturation_equals_rendered_red(
            saturation in 0u32..=100_000,
        ) {
            let saturation_factor = f64::from(saturation) / 100_000.0;
            let red = (0.5 * (1.0 + saturation_factor) * 255.0 + 1e-9).round() as u8;
            let other = (0.5 * (1.0 - saturation_factor) * 255.0 + 1e-9).round() as u8;
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:sat val="{saturation}"/></a:schemeClr>"#
            );
            let direct_color = format!(r#"<a:srgbClr val="{red:02X}{other:02X}{other:02X}"/>"#);
            let theme = test_theme("FF0000", "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_absolute_luminance_equals_rendered_gray(
            luminance in 0u32..=100_000,
        ) {
            let channel = (f64::from(luminance) / 100_000.0 * 255.0 + 1e-9).round() as u8;
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:lum val="{luminance}"/></a:schemeClr>"#
            );
            let direct_color = format!(r#"<a:srgbClr val="{channel:02X}{channel:02X}{channel:02X}"/>"#);
            let theme = test_theme("808080", "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn ordered_drawingml_absolute_luminance_and_modulation_equal_rendered_gray(
            channel in any::<u8>(),
            luminance in 0u32..=100_000,
            modulation in 0u32..=200_000,
            absolute_first in any::<bool>(),
        ) {
            let absolute = f64::from(luminance) / 100_000.0;
            let factor = f64::from(modulation) / 100_000.0;
            let rendered_luminance = if absolute_first {
                (absolute * factor).clamp(0.0, 1.0)
            } else {
                absolute
            };
            let rendered = (rendered_luminance * 255.0 + 1e-9).round() as u8;
            let (first, first_value, second, second_value) = if absolute_first {
                ("lum", luminance, "lumMod", modulation)
            } else {
                ("lumMod", modulation, "lum", luminance)
            };
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:{first} val="{first_value}"/><a:{second} val="{second_value}"/></a:schemeClr>"#
            );
            let direct_color = format!(r#"<a:srgbClr val="{rendered:02X}{rendered:02X}{rendered:02X}"/>"#);
            let base = format!("{channel:02X}{channel:02X}{channel:02X}");
            let theme = test_theme(&base, "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_inverse_and_complement_equal_rendered_rgb(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            inverse in any::<bool>(),
        ) {
            let channels = [red, green, blue];
            let rendered = if inverse {
                channels.map(|channel| 255 - channel)
            } else {
                let sum = u16::from(channels.iter().copied().min().unwrap())
                    + u16::from(channels.iter().copied().max().unwrap());
                channels.map(|channel| (sum - u16::from(channel)) as u8)
            };
            let transform = if inverse { "inv" } else { "comp" };
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:{transform}/></a:schemeClr>"#
            );
            let direct_color = format!(
                r#"<a:srgbClr val="{:02X}{:02X}{:02X}"/>"#,
                rendered[0], rendered[1], rendered[2]
            );
            let theme = test_theme(&format!("{red:02X}{green:02X}{blue:02X}"), "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn paired_drawingml_inverse_and_complement_are_identity(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            inverse in any::<bool>(),
        ) {
            let transform = if inverse { "inv" } else { "comp" };
            let transformed_color = format!(
                r#"<a:srgbClr val="{red:02X}{green:02X}{blue:02X}"><a:{transform}/><a:{transform}/></a:srgbClr>"#
            );
            let direct_color = format!(
                r#"<a:srgbClr val="{red:02X}{green:02X}{blue:02X}"/>"#
            );
            let theme = test_theme("000000", "FFFFFF");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &transformed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn ordered_drawingml_alpha_transforms_equal_explicit_alpha(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            modulation in 0u32..=200_000,
            offset in -100_000i32..=100_000,
            offset_first in any::<bool>(),
        ) {
            let modulation_factor = f64::from(modulation) / 100_000.0;
            let offset_amount = f64::from(offset) / 100_000.0;
            let opacity = if offset_first {
                (1.0 + offset_amount).clamp(0.0, 1.0) * modulation_factor
            } else {
                (1.0 * modulation_factor).clamp(0.0, 1.0) + offset_amount
            }
            .clamp(0.0, 1.0);
            let opacity = (opacity * 100_000.0 + 1e-9).round() as u32;
            let (first, first_value, second, second_value) = if offset_first {
                ("alphaOff", offset.to_string(), "alphaMod", modulation.to_string())
            } else {
                ("alphaMod", modulation.to_string(), "alphaOff", offset.to_string())
            };
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:{first} val="{first_value}"/><a:{second} val="{second_value}"/></a:schemeClr>"#
            );
            let direct_color = format!(
                r#"<a:srgbClr val="{red:02X}{green:02X}{blue:02X}"><a:alpha val="{opacity}"/></a:srgbClr>"#
            );
            let theme = test_theme(&format!("{red:02X}{green:02X}{blue:02X}"), "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_theme_and_reference_alpha_transforms_compose(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            theme_opacity in 0u32..=100_000,
            shade in 0u32..=100_000,
            modulation in 0u32..=200_000,
            offset in -100_000i32..=100_000,
        ) {
            let opacity = (((f64::from(theme_opacity) / 100_000.0)
                * (f64::from(modulation) / 100_000.0))
                .clamp(0.0, 1.0)
                + f64::from(offset) / 100_000.0)
                .clamp(0.0, 1.0);
            let opacity = (opacity * 100_000.0 + 1e-9).round() as u32;
            let root = xml::parse(
                format!(
                    r#"<a:theme xmlns:a="a"><a:themeElements><a:clrScheme name="Office"><a:accent1><a:srgbClr val="{red:02X}{green:02X}{blue:02X}"><a:alpha val="{theme_opacity}"/></a:srgbClr></a:accent1></a:clrScheme></a:themeElements></a:theme>"#
                )
                .as_bytes(),
            )
            .unwrap();
            let mut theme = PptxTheme::default();
            theme.merge_theme(&root);
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:shade val="{shade}"/><a:alphaMod val="{modulation}"/><a:alphaOff val="{offset}"/></a:schemeClr>"#
            );
            let rgb = format!("{red:02X}{green:02X}{blue:02X}");
            let rendered = rendered_tint_or_shade(&rgb, shade, false);
            let direct_color = format!(
                r#"<a:srgbClr val="{rendered}"><a:alpha val="{opacity}"/></a:srgbClr>"#
            );
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_alpha_changes_are_reported(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            old_opacity in 0u32..=100_000,
            new_opacity in 0u32..=100_000,
        ) {
            prop_assume!(old_opacity != new_opacity);
            let old_color = format!(
                r#"<a:srgbClr val="{red:02X}{green:02X}{blue:02X}"><a:alpha val="{old_opacity}"/></a:srgbClr>"#
            );
            let new_color = format!(
                r#"<a:srgbClr val="{red:02X}{green:02X}{blue:02X}"><a:alpha val="{new_opacity}"/></a:srgbClr>"#
            );
            let theme = test_theme("000000", "FFFFFF");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &old_color)),
                &presentation(themed_text_shape(&theme, &new_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn explicit_opaque_drawingml_alpha_is_default(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
        ) {
            let with_alpha = format!(
                r#"<a:srgbClr val="{red:02X}{green:02X}{blue:02X}"><a:alpha val="100000"/></a:srgbClr>"#
            );
            let without_alpha = format!(
                r#"<a:srgbClr val="{red:02X}{green:02X}{blue:02X}"/>"#
            );
            let theme = test_theme("000000", "FFFFFF");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &with_alpha)),
                &presentation(themed_text_shape(&theme, &without_alpha)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_channel_transforms_equal_rendered_rgb(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            channel in 0usize..3,
            operation in 0usize..3,
            absolute in 0u32..=100_000,
            modulation in 0u32..=200_000,
            offset in -100_000i32..=100_000,
        ) {
            let transform_names = [
                ["red", "redMod", "redOff"],
                ["green", "greenMod", "greenOff"],
                ["blue", "blueMod", "blueOff"],
            ];
            let mut rendered = [red, green, blue];
            let (amount, value) = match operation {
                0 => (absolute.to_string(), f64::from(absolute) / 100_000.0),
                1 => (
                    modulation.to_string(),
                    f64::from(rendered[channel]) / 255.0
                        * f64::from(modulation) / 100_000.0,
                ),
                _ => (
                    offset.to_string(),
                    f64::from(rendered[channel]) / 255.0
                        + f64::from(offset) / 100_000.0,
                ),
            };
            rendered[channel] = (value.clamp(0.0, 1.0) * 255.0 + 1e-9).round() as u8;
            let transform = transform_names[channel][operation];
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:{transform} val="{amount}"/></a:schemeClr>"#
            );
            let direct_color = format!(
                r#"<a:srgbClr val="{:02X}{:02X}{:02X}"/>"#,
                rendered[0], rendered[1], rendered[2]
            );
            let theme = test_theme(&format!("{red:02X}{green:02X}{blue:02X}"), "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn ordered_drawingml_channel_transforms_equal_rendered_rgb(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            channel in 0usize..3,
            modulation in 0u32..=200_000,
            offset in -100_000i32..=100_000,
        ) {
            let names = [["redMod", "redOff"], ["greenMod", "greenOff"], ["blueMod", "blueOff"]];
            let mut rendered = [red, green, blue];
            let value = (f64::from(rendered[channel]) / 255.0
                * f64::from(modulation) / 100_000.0)
                .clamp(0.0, 1.0);
            let value = (value + f64::from(offset) / 100_000.0).clamp(0.0, 1.0);
            rendered[channel] = (value * 255.0 + 1e-9).round() as u8;
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:{} val="{modulation}"/><a:{} val="{offset}"/></a:schemeClr>"#,
                names[channel][0], names[channel][1]
            );
            let direct_color = format!(
                r#"<a:srgbClr val="{:02X}{:02X}{:02X}"/>"#,
                rendered[0], rendered[1], rendered[2]
            );
            let theme = test_theme(&format!("{red:02X}{green:02X}{blue:02X}"), "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn drawingml_hue_offsets_follow_the_six_color_wheel(
            step in 0usize..6,
        ) {
            let expected = ["FF0000", "FFFF00", "00FF00", "00FFFF", "0000FF", "FF00FF"][step];
            let offset = step as i32 * 3_600_000;
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:hueOff val="{offset}"/></a:schemeClr>"#
            );
            let direct_color = format!(r#"<a:srgbClr val="{expected}"/>"#);
            let theme = test_theme("FF0000", "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn drawingml_hue_modulation_follows_the_six_color_wheel(
            step in 0usize..6,
        ) {
            let expected = ["FF0000", "FFFF00", "00FF00", "00FFFF", "0000FF", "FF00FF"][step];
            let modulation = step as u32 * 25_000;
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:hueMod val="{modulation}"/></a:schemeClr>"#
            );
            let direct_color = format!(r#"<a:srgbClr val="{expected}"/>"#);
            let theme = test_theme("0000FF", "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_saturation_transforms_equal_rendered_red(
            modulation in 0u32..=200_000,
            offset in -100_000i32..=100_000,
            use_modulation in any::<bool>(),
        ) {
            let (transform, amount, saturation) = if use_modulation {
                (
                    "satMod",
                    modulation.to_string(),
                    f64::from(modulation) / 100_000.0,
                )
            } else {
                (
                    "satOff",
                    offset.to_string(),
                    1.0 + f64::from(offset) / 100_000.0,
                )
            };
            let saturation = saturation.clamp(0.0, 1.0);
            let red = (0.5 * (1.0 + saturation) * 255.0 + 1e-9).round() as u8;
            let other = (0.5 * (1.0 - saturation) * 255.0 + 1e-9).round() as u8;
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:{transform} val="{amount}"/></a:schemeClr>"#
            );
            let direct_color = format!(r#"<a:srgbClr val="{red:02X}{other:02X}{other:02X}"/>"#);
            let theme = test_theme("FF0000", "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_luminance_transforms_equal_rendered_gray(
            channel in any::<u8>(),
            modulation in 0u32..=200_000,
            offset in -100_000i32..=100_000,
            use_modulation in any::<bool>(),
        ) {
            let luminance = f64::from(channel) / 255.0;
            let (transform, amount, rendered_luminance) = if use_modulation {
                (
                    "lumMod",
                    modulation.to_string(),
                    luminance * f64::from(modulation) / 100_000.0,
                )
            } else {
                (
                    "lumOff",
                    offset.to_string(),
                    luminance + f64::from(offset) / 100_000.0,
                )
            };
            let rendered = (rendered_luminance.clamp(0.0, 1.0) * 255.0 + 1e-9).round() as u8;
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:{transform} val="{amount}"/></a:schemeClr>"#
            );
            let direct_color = format!(r#"<a:srgbClr val="{rendered:02X}{rendered:02X}{rendered:02X}"/>"#);
            let base = format!("{channel:02X}{channel:02X}{channel:02X}");
            let theme = test_theme(&base, "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn ordered_drawingml_luminance_transforms_equal_rendered_gray(
            channel in any::<u8>(),
            modulation in 0u32..=200_000,
            offset in -100_000i32..=100_000,
        ) {
            let luminance = (f64::from(channel) / 255.0 * f64::from(modulation) / 100_000.0)
                .clamp(0.0, 1.0);
            let luminance =
                (luminance + f64::from(offset) / 100_000.0).clamp(0.0, 1.0);
            let rendered = (luminance * 255.0 + 1e-9).round() as u8;
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:lumMod val="{modulation}"/><a:lumOff val="{offset}"/></a:schemeClr>"#
            );
            let direct_color = format!(r#"<a:srgbClr val="{rendered:02X}{rendered:02X}{rendered:02X}"/>"#);
            let base = format!("{channel:02X}{channel:02X}{channel:02X}");
            let theme = test_theme(&base, "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn ordered_drawingml_rgb_transforms_equal_rendered_rgb(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            first_amount in 0u32..=100_000,
            second_amount in 0u32..=100_000,
            tint_first in any::<bool>(),
        ) {
            let apply = |channel: f64, amount: u32, tint: bool| {
                let factor = f64::from(amount) / 100_000.0;
                if tint {
                    channel * factor + 255.0 * (1.0 - factor)
                } else {
                    channel * factor
                }
            };
            let render = |channel: u8| {
                apply(
                    apply(f64::from(channel), first_amount, tint_first),
                    second_amount,
                    !tint_first,
                )
                .round() as u8
            };
            let rendered = format!("{:02X}{:02X}{:02X}", render(red), render(green), render(blue));
            let first = if tint_first { "tint" } else { "shade" };
            let second = if tint_first { "shade" } else { "tint" };
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:{first} val="{first_amount}"/><a:{second} val="{second_amount}"/></a:schemeClr>"#
            );
            let direct_color = format!(r#"<a:srgbClr val="{rendered}"/>"#);
            let theme = test_theme(&format!("{red:02X}{green:02X}{blue:02X}"), "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_drawingml_theme_tints_and_shades_equal_rendered_rgb(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            amount in 0u32..=100_000,
            tint in any::<bool>(),
        ) {
            let factor = f64::from(amount) / 100_000.0;
            let render = |channel: u8| {
                let channel = f64::from(channel);
                let rendered = if tint {
                    channel * factor + 255.0 * (1.0 - factor)
                } else {
                    channel * factor
                };
                rendered.round() as u8
            };
            let rendered = format!("{:02X}{:02X}{:02X}", render(red), render(green), render(blue));
            let transform = if tint { "tint" } else { "shade" };
            let themed_color = format!(
                r#"<a:schemeClr val="accent1"><a:{transform} val="{amount}"/></a:schemeClr>"#
            );
            let direct_color = format!(r#"<a:srgbClr val="{rendered}"/>"#);
            let theme = test_theme(&format!("{red:02X}{green:02X}{blue:02X}"), "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(&theme, &themed_color)),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn equivalent_integer_spellings_have_one_table_cell_margin(
            value in 0i64..=1_000_000,
            padding in 1usize..5,
            explicit_plus in any::<bool>(),
        ) {
            fn table(margin: &str) -> Node {
                xml::parse(
                    format!(r#"<a:tbl xmlns:a="a"><a:tr><a:tc><a:tcPr marL="{margin}"/></a:tc></a:tr></a:tbl>"#).as_bytes(),
                )
                .unwrap()
            }

            let sign = if explicit_plus { "+" } else { "" };
            let padded = format!("{sign}{}{value}", "0".repeat(padding));

            prop_assert_eq!(
                table_appearance_signature(&table(&value.to_string()), &PptxTheme::default()),
                table_appearance_signature(&table(&padded), &PptxTheme::default()),
            );
        }

        #[test]
        fn equivalent_integer_spellings_have_one_shape_transform(
            value in 0i64..=21_600_000,
            padding in 1usize..5,
            explicit_plus in any::<bool>(),
        ) {
            let sign = if explicit_plus { "+" } else { "" };
            let padded = format!("{sign}{}{value}", "0".repeat(padding));
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(text_shape(&format!(r#"rot="{value}""#), "")),
                &presentation(text_shape(&format!(r#"rot="{padded}""#), "")),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn equivalent_integer_spellings_have_one_run_style(
            value in -100_000i64..=100_000,
            padding in 1usize..5,
            explicit_plus in any::<bool>(),
        ) {
            let canonical = value.to_string();
            let sign = if value < 0 {
                "-"
            } else if explicit_plus {
                "+"
            } else {
                ""
            };
            let padded = format!("{sign}{}{value}", "0".repeat(padding), value = value.abs());
            let canonical_node = xml::parse(
                format!(r#"<a:rPr xmlns:a="a" baseline="{canonical}"/>"#).as_bytes(),
            )
            .unwrap();
            let padded_node = xml::parse(
                format!(r#"<a:rPr xmlns:a="a" baseline="{padded}"/>"#).as_bytes(),
            )
            .unwrap();

            prop_assert_eq!(
                run_style_signature(None, Some(&canonical_node), None),
                run_style_signature(None, Some(&padded_node), None),
            );
        }

        #[test]
        fn arbitrary_theme_and_direct_text_colors_are_equivalent(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            lowercase_theme in any::<bool>(),
        ) {
            let rgb = format!("{red:02X}{green:02X}{blue:02X}");
            let theme_rgb = if lowercase_theme {
                rgb.to_ascii_lowercase()
            } else {
                rgb.clone()
            };
            let theme = test_theme(&theme_rgb, "000000");
            let direct_color = format!(r#"<a:srgbClr val="{rgb}"/>"#);
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(
                    &theme,
                    r#"<a:schemeClr val="accent1"/>"#,
                )),
                &presentation(themed_text_shape(&theme, &direct_color)),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert!(!diff.has_changes(), "{diff:?}");
        }

        #[test]
        fn arbitrary_referenced_theme_text_color_changes_are_reported(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
        ) {
            let old_rgb = format!("{red:02X}{green:02X}{blue:02X}");
            let new_rgb = format!("{:02X}{:02X}{:02X}", !red, !green, !blue);
            let old_theme = test_theme(&old_rgb, "000000");
            let new_theme = test_theme(&new_rgb.to_ascii_lowercase(), "000000");
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &presentation(themed_text_shape(
                    &old_theme,
                    r#"<a:schemeClr val="accent1"/>"#,
                )),
                &presentation(themed_text_shape(
                    &new_theme,
                    r#"<a:schemeClr val="accent1"/>"#,
                )),
                &crate::diff::DiffOptions::default(),
            );

            prop_assert_eq!(diff.summary.paragraphs_changed, 1, "{:?}", diff);
        }

        #[test]
        fn office_math_changes_are_reported(
            base in "[A-Za-z]{1,8}",
            exponent in 0u16..1000,
        ) {
            let old = presentation(equation_shape(&base, exponent));
            let new = presentation(equation_shape(&base, exponent + 1000));
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &old,
                &new,
                &crate::diff::DiffOptions::default(),
            );
            prop_assert_eq!(diff.summary.slides_modified, 1);
            prop_assert_eq!(diff.summary.paragraphs_changed, 1);
            let paragraph = &diff.slides[0].shapes[0].paragraphs[0];
            prop_assert_eq!(paragraph.kind, crate::diff::ChangeKind::Changed);
        }

        #[test]
        fn chart_value_changes_are_reported(value in 0u16..10_000, delta in 1u16..1000) {
            let old = load_presentation(&chart_pptx(value), &Limits::default()).unwrap();
            let new = load_presentation(
                &chart_pptx(value.saturating_add(delta)),
                &Limits::default(),
            )
            .unwrap();
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &old,
                &new,
                &crate::diff::DiffOptions::default(),
            );
            prop_assert_eq!(diff.summary.slides_modified, 1);
            prop_assert_eq!(diff.summary.paragraphs_changed, 1);
        }

        #[test]
        fn smartart_label_changes_are_reported(value in 0u16..10_000, delta in 1u16..1000) {
            let old = load_presentation(&smartart_pptx(value), &Limits::default()).unwrap();
            let new = load_presentation(
                &smartart_pptx(value.saturating_add(delta)),
                &Limits::default(),
            )
            .unwrap();
            let diff = crate::diff::diff_presentations(
                "old.pptx",
                "new.pptx",
                &old,
                &new,
                &crate::diff::DiffOptions::default(),
            );
            prop_assert_eq!(diff.summary.slides_modified, 1);
            prop_assert_eq!(diff.summary.paragraphs_changed, 1);
        }
    }
}
