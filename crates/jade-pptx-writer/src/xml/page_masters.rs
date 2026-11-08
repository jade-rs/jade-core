//! ノート マスター and 配布資料マスター: made when a deck has none (PowerPoint
//! requires a notes master behind every notes slide), their ヘッダーとフッター
//! (`<p:hf>` and the header, date and footer placeholders' text) and the
//! orientation of the page they share (`<p:notesSz>`).

use super::presenting::{
    add_content_type, add_relationship, escape, insert_ordered, prefix, presentation_part, related,
};
use super::slides::{fresh_part, p_ns, r_ns};
use super::{apply_patches, child, drawing_ns, edit_tag, parse_document_nodes, xml_error, Node};
use crate::Result;
use jade_opc::{rels_part_name, OpcPackage};
use serde::{Deserialize, Serialize};

const NOTES_MASTER_MIME: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.notesMaster+xml";
const HANDOUT_MASTER_MIME: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.handoutMaster+xml";
const THEME_MIME: &str = "application/vnd.openxmlformats-officedocument.theme+xml";
/// The `CT_NotesMaster` / `CT_HandoutMaster` children after `cSld`.
const MASTER_ORDER: &[&str] = &["cSld", "clrMap", "hf", "notesStyle", "extLst"];

/// ヘッダーとフッター › ノートと配布資料: what the notes pages and handouts
/// show in their corners.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PageHeaderFooter {
    /// 日付と時刻; `date_text` is 固定's text, `None` for 自動更新.
    pub date: bool,
    pub date_text: Option<String>,
    /// ヘッダー and its text.
    pub header: Option<String>,
    /// ページ番号.
    pub page_number: bool,
    /// フッター and its text.
    pub footer: Option<String>,
}

/// One placeholder of a page master: (type, idx, name, x, y, cx, cy).
type Slot = (&'static str, u32, &'static str, i64, i64, i64, i64);

/// PowerPoint's notes master on a 7.5 × 10 in portrait page.
const NOTES_SLOTS: &[Slot] = &[
    ("hdr", 0, "Header Placeholder", 0, 0, 2_971_800, 458_788),
    (
        "dt",
        1,
        "Date Placeholder",
        3_884_613,
        0,
        2_971_800,
        458_788,
    ),
    (
        "sldImg",
        2,
        "Slide Image Placeholder",
        685_800,
        1_143_000,
        5_486_400,
        3_086_100,
    ),
    (
        "body",
        3,
        "Notes Placeholder",
        685_800,
        4_400_550,
        5_486_400,
        3_600_450,
    ),
    (
        "ftr",
        4,
        "Footer Placeholder",
        0,
        8_685_213,
        2_971_800,
        458_787,
    ),
    (
        "sldNum",
        5,
        "Slide Number Placeholder",
        3_884_613,
        8_685_213,
        2_971_800,
        458_787,
    ),
];
const HANDOUT_SLOTS: &[Slot] = &[
    ("hdr", 0, "Header Placeholder", 0, 0, 2_971_800, 458_788),
    (
        "dt",
        1,
        "Date Placeholder",
        3_884_613,
        0,
        2_971_800,
        458_788,
    ),
    (
        "ftr",
        2,
        "Footer Placeholder",
        0,
        8_685_213,
        2_971_800,
        458_787,
    ),
    (
        "sldNum",
        3,
        "Slide Number Placeholder",
        3_884_613,
        8_685_213,
        2_971_800,
        458_787,
    ),
];
const NOTES_PAGE: (i64, i64) = (6_858_000, 9_144_000);

fn placeholder_xml(slot: &Slot, id: u32, scale: (f64, f64), slide_aspect: f64) -> String {
    let &(kind, idx, name, x, y, cx, cy) = slot;
    let (sx, sy) = scale;
    let (mut x, mut y, mut cx, mut cy) = (
        (x as f64 * sx) as i64,
        (y as f64 * sy) as i64,
        (cx as f64 * sx) as i64,
        (cy as f64 * sy) as i64,
    );
    if kind == "sldImg" {
        // The slide's own proportions, centred in its box.
        let (box_x, box_y, box_w, box_h) = (x, y, cx, cy);
        if box_w as f64 / box_h as f64 > slide_aspect {
            cx = (box_h as f64 * slide_aspect) as i64;
            cy = box_h;
        } else {
            cx = box_w;
            cy = (box_w as f64 / slide_aspect) as i64;
        }
        x = box_x + (box_w - cx) / 2;
        y = box_y + (box_h - cy) / 2;
    }
    let ph = match kind {
        "hdr" => "<p:ph type=\"hdr\" sz=\"quarter\"/>".to_string(),
        "sldImg" => format!("<p:ph type=\"sldImg\" idx=\"{idx}\"/>"),
        "body" => format!("<p:ph type=\"body\" sz=\"quarter\" idx=\"{idx}\"/>"),
        "dt" => format!("<p:ph type=\"dt\" idx=\"{idx}\"/>"),
        other => format!("<p:ph type=\"{other}\" sz=\"quarter\" idx=\"{idx}\"/>"),
    };
    let locks = if kind == "sldImg" {
        "<a:spLocks noGrp=\"1\" noRot=\"1\" noChangeAspect=\"1\"/>"
    } else {
        "<a:spLocks noGrp=\"1\"/>"
    };
    let geometry = format!(
        "<a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>"
    );
    let right = matches!(kind, "dt" | "sldNum");
    let bottom = matches!(kind, "ftr" | "sldNum");
    let text = match kind {
        "sldImg" => String::new(),
        "body" => "<p:txBody><a:bodyPr vert=\"horz\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" rtlCol=\"0\"/><a:lstStyle/><a:p><a:pPr lvl=\"0\"/><a:r><a:rPr lang=\"ja-JP\" altLang=\"en-US\"/><a:t>マスター テキストの書式設定</a:t></a:r></a:p><a:p><a:pPr lvl=\"1\"/><a:r><a:rPr lang=\"ja-JP\" altLang=\"en-US\"/><a:t>第 2 レベル</a:t></a:r></a:p><a:p><a:pPr lvl=\"2\"/><a:r><a:rPr lang=\"ja-JP\" altLang=\"en-US\"/><a:t>第 3 レベル</a:t></a:r></a:p><a:p><a:pPr lvl=\"3\"/><a:r><a:rPr lang=\"ja-JP\" altLang=\"en-US\"/><a:t>第 4 レベル</a:t></a:r></a:p><a:p><a:pPr lvl=\"4\"/><a:r><a:rPr lang=\"ja-JP\" altLang=\"en-US\"/><a:t>第 5 レベル</a:t></a:r></a:p></p:txBody>".to_string(),
        _ => {
            let content = match kind {
                "dt" => "<a:fld id=\"{7C5F1E7B-0E47-4A54-9B5B-6E5D3A9D2A11}\" type=\"datetimeFigureOut\"><a:rPr lang=\"ja-JP\" altLang=\"en-US\"/><a:t>2026/11/2</a:t></a:fld>".to_string(),
                "sldNum" => "<a:fld id=\"{0F3E5D7A-2B1C-4C8D-9E6F-1A2B3C4D5E6F}\" type=\"slidenum\"><a:rPr lang=\"ja-JP\" altLang=\"en-US\"/><a:t>‹#›</a:t></a:fld>".to_string(),
                _ => String::new(),
            };
            format!(
                "<p:txBody><a:bodyPr vert=\"horz\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" rtlCol=\"0\"{}/><a:lstStyle><a:lvl1pPr algn=\"{}\"><a:defRPr sz=\"1200\"/></a:lvl1pPr></a:lstStyle><a:p>{content}<a:endParaRPr lang=\"ja-JP\" altLang=\"en-US\"/></a:p></p:txBody>",
                if bottom { " anchor=\"b\"" } else { "" },
                if right { "r" } else { "l" }
            )
        }
    };
    let outline = if kind == "sldImg" {
        "<a:noFill/><a:ln w=\"12700\"><a:solidFill><a:prstClr val=\"black\"/></a:solidFill></a:ln>"
    } else {
        ""
    };
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name} {}\"/><p:cNvSpPr>{locks}</p:cNvSpPr><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:spPr>{geometry}{outline}</p:spPr>{text}</p:sp>",
        id - 1
    )
}

const CLR_MAP: &str = "<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>";

fn notes_style() -> String {
    let mut style = String::from("<p:notesStyle>");
    for level in 1..=9 {
        let margin = (level - 1) * 457_200;
        style.push_str(&format!(
            "<a:lvl{level}pPr marL=\"{margin}\" algn=\"l\" defTabSz=\"914400\" rtl=\"0\" eaLnBrk=\"1\" latinLnBrk=\"0\" hangingPunct=\"1\"><a:defRPr kumimoji=\"1\" sz=\"1200\" kern=\"1200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/><a:ea typeface=\"+mn-ea\"/><a:cs typeface=\"+mn-cs\"/></a:defRPr></a:lvl{level}pPr>"
        ));
    }
    style.push_str("</p:notesStyle>");
    style
}

/// A page master's XML: its placeholders scaled from the portrait default
/// onto `page`.
fn master_xml(notes: bool, page: (i64, i64), slide_aspect: f64, strict: bool) -> String {
    let root = if notes {
        "notesMaster"
    } else {
        "handoutMaster"
    };
    let slots = if notes { NOTES_SLOTS } else { HANDOUT_SLOTS };
    let scale = (
        page.0 as f64 / NOTES_PAGE.0 as f64,
        page.1 as f64 / NOTES_PAGE.1 as f64,
    );
    let shapes: String = slots
        .iter()
        .enumerate()
        .map(|(i, slot)| placeholder_xml(slot, i as u32 + 2, scale, slide_aspect))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<p:{root} xmlns:a=\"{}\" xmlns:r=\"{}\" xmlns:p=\"{}\"><p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>{shapes}</p:spTree></p:cSld>{CLR_MAP}{}</p:{root}>",
        drawing_ns(strict),
        r_ns(strict),
        p_ns(strict),
        if notes { notes_style() } else { String::new() }
    )
}

/// The page size and the slide's proportions, from `presentation.xml`.
fn sizes(nodes: &[Node]) -> ((i64, i64), f64) {
    let read = |name: &str| {
        let node = child(nodes, &nodes[0], 1, name)?;
        Some((
            node.attr("cx")?.parse::<i64>().ok()?,
            node.attr("cy")?.parse::<i64>().ok()?,
        ))
    };
    let page = read("notesSz")
        .filter(|(w, h)| *w > 0 && *h > 0)
        .unwrap_or(NOTES_PAGE);
    let slide = read("sldSz")
        .filter(|(w, h)| *w > 0 && *h > 0)
        .unwrap_or((12_192_000, 6_858_000));
    (page, slide.0 as f64 / slide.1 as f64)
}

/// Copies the first slide master's theme for a new page master to use,
/// returning the copy's part name.
fn copy_theme(package: &mut OpcPackage, presentation: &str) -> Result<Option<String>> {
    let Some(master) = related(package, presentation, "slideMaster")? else {
        return Ok(None);
    };
    let Some(theme) = related(package, &master, "theme")? else {
        return Ok(None);
    };
    let Some(bytes) = package.part(&theme).map(<[u8]>::to_vec) else {
        return Ok(None);
    };
    let copy = {
        let (stem, _) = theme.rsplit_once('/').unwrap_or(("ppt/theme", ""));
        (1..)
            .map(|i| format!("{stem}/theme{i}.xml"))
            .find(|name| package.part(name).is_none())
            .expect("a free theme name")
    };
    package.replace(&copy, bytes);
    if let Some(rels) = package.part(&rels_part_name(&theme)).map(<[u8]>::to_vec) {
        package.replace(&rels_part_name(&copy), rels);
    }
    add_content_type(package, &copy, THEME_MIME)?;
    Ok(Some(copy))
}

/// The deck's notes (`notes`) or handout master, made when it has none.
/// Returns the new bytes and the master's part name.
fn ensure_master(bytes: &[u8], notes: bool) -> Result<(Vec<u8>, String)> {
    let kind = if notes {
        "notesMaster"
    } else {
        "handoutMaster"
    };
    let mut package = OpcPackage::open_preserving(bytes)?;
    let presentation = presentation_part(&package)?;
    if let Some(part) = related(&package, &presentation, kind)? {
        if package.part(&part).is_some() {
            return Ok((bytes.to_vec(), part));
        }
    }
    let xml = package.required_part_str(&presentation)?;
    let nodes = parse_document_nodes(&xml)?;
    let root = nodes
        .first()
        .filter(|n| n.is(1, "presentation"))
        .ok_or_else(|| xml_error("expected a presentation"))?;
    let strict = root.strict;
    let (page, aspect) = sizes(&nodes);
    let directory = if notes {
        "ppt/notesMasters/notesMaster1.xml"
    } else {
        "ppt/handoutMasters/handoutMaster1.xml"
    };
    let part = if package.part(directory).is_none() {
        directory.to_string()
    } else {
        fresh_part(&package, directory)
    };
    package.replace(&part, master_xml(notes, page, aspect, strict).into_bytes());
    add_content_type(
        &mut package,
        &part,
        if notes {
            NOTES_MASTER_MIME
        } else {
            HANDOUT_MASTER_MIME
        },
    )?;
    if let Some(theme) = copy_theme(&mut package, &presentation)? {
        add_relationship(&mut package, &part, "theme", &theme, strict)?;
    }
    let id = add_relationship(&mut package, &presentation, kind, &part, strict)?;

    // `<p:notesMasterIdLst>` / `<p:handoutMasterIdLst>` name it.
    let p = prefix(root);
    let list = format!(
        "<{p}{kind}IdLst xmlns:r=\"{}\"><{p}{kind}Id r:id=\"{id}\"/></{p}{kind}IdLst>",
        r_ns(strict)
    );
    let updated = insert_ordered(
        &xml,
        &nodes,
        root,
        &format!("{kind}IdLst"),
        &list,
        super::presenting::PRESENTATION_ORDER,
    );
    package.replace(&presentation, updated.into_bytes());

    // Notes slides made without a master get this one.
    if notes {
        let orphans: Vec<String> = package
            .part_names()
            .iter()
            .filter(|name| name.contains("notesSlides/") && name.ends_with(".xml"))
            .filter(|name| !name.contains("/_rels/"))
            .cloned()
            .collect();
        for notes_slide in orphans {
            if related(&package, &notes_slide, "notesMaster")?.is_none() {
                add_relationship(&mut package, &notes_slide, "notesMaster", &part, strict)?;
            }
        }
    }
    Ok((package.write()?, part))
}

/// Makes the notes master when the deck has none.
pub(crate) fn ensure_notes_master(bytes: &[u8]) -> Result<Vec<u8>> {
    Ok(ensure_master(bytes, true)?.0)
}

/// Makes the handout master when the deck has none.
pub(crate) fn ensure_handout_master(bytes: &[u8]) -> Result<Vec<u8>> {
    Ok(ensure_master(bytes, false)?.0)
}

/// The placeholders of `kind` in a master's shape tree.
fn placeholders<'a>(nodes: &'a [Node], kind: &str) -> Vec<&'a Node> {
    nodes
        .iter()
        .filter(|n| n.is(1, "sp"))
        .filter(|sp| {
            child(nodes, sp, 1, "nvSpPr")
                .and_then(|nv| child(nodes, nv, 1, "nvPr"))
                .and_then(|nv| child(nodes, nv, 1, "ph"))
                .and_then(|ph| ph.attr("type"))
                == Some(kind)
        })
        .collect()
}

/// A placeholder's paragraphs replaced by one holding `content` (already
/// XML), its `bodyPr` and `lstStyle` kept.
fn replace_paragraphs(
    nodes: &[Node],
    shape: &Node,
    content: &str,
) -> Option<(std::ops::Range<usize>, String)> {
    let body = child(nodes, shape, 1, "txBody")?;
    let paragraphs: Vec<&Node> = body
        .children
        .iter()
        .map(|&i| &nodes[i])
        .filter(|n| n.is(2, "p"))
        .collect();
    let a = drawing_prefix(nodes, body);
    let paragraph = format!("<{a}p>{content}</{a}p>");
    match (paragraphs.first(), paragraphs.last()) {
        (Some(first), Some(last)) => Some((first.outer.start..last.outer.end, paragraph)),
        _ => Some((body.range.end..body.range.end, paragraph)),
    }
}

/// The prefix DrawingML elements use under `body` (`a:` almost always).
fn drawing_prefix(nodes: &[Node], body: &Node) -> String {
    body.children
        .iter()
        .map(|&i| &nodes[i])
        .find(|n| n.ns == 2)
        .map(prefix)
        .unwrap_or_else(|| "a:".into())
}

fn run(a: &str, text: &str) -> String {
    format!(
        "<{a}r><{a}rPr lang=\"ja-JP\" altLang=\"en-US\"/><{a}t>{}</{a}t></{a}r>",
        escape(text)
    )
}

/// Applies `settings` to one master part.
fn apply_header_footer(xml: &str, settings: &PageHeaderFooter) -> Result<String> {
    let nodes = parse_document_nodes(xml)?;
    let root = &nodes[0];
    let p = prefix(root);
    let mut patches = Vec::new();
    for (kind, text) in [("hdr", &settings.header), ("ftr", &settings.footer)] {
        if let Some(text) = text {
            for shape in placeholders(&nodes, kind) {
                let a = child(&nodes, shape, 1, "txBody")
                    .map(|body| drawing_prefix(&nodes, body))
                    .unwrap_or_else(|| "a:".into());
                if let Some(patch) = replace_paragraphs(&nodes, shape, &run(&a, text)) {
                    patches.push(patch);
                }
            }
        }
    }
    if settings.date {
        for shape in placeholders(&nodes, "dt") {
            let a = child(&nodes, shape, 1, "txBody")
                .map(|body| drawing_prefix(&nodes, body))
                .unwrap_or_else(|| "a:".into());
            let content = match &settings.date_text {
                Some(fixed) => run(&a, fixed),
                None => format!(
                    "<{a}fld id=\"{{7C5F1E7B-0E47-4A54-9B5B-6E5D3A9D2A11}}\" type=\"datetime1\"><{a}rPr lang=\"ja-JP\" altLang=\"en-US\"/><{a}t>2026/11/2</{a}t></{a}fld>"
                ),
            };
            if let Some(patch) = replace_paragraphs(&nodes, shape, &content) {
                patches.push(patch);
            }
        }
    }
    let flag = |on: bool| if on { "1" } else { "0" };
    let hf = format!(
        "<{p}hf hdr=\"{}\" ftr=\"{}\" dt=\"{}\" sldNum=\"{}\"/>",
        flag(settings.header.is_some()),
        flag(settings.footer.is_some()),
        flag(settings.date),
        flag(settings.page_number)
    );
    match child(&nodes, root, 1, "hf") {
        Some(existing) => patches.push((existing.outer.clone(), hf)),
        None => {
            let updated = apply_patches(xml, patches);
            let nodes = parse_document_nodes(&updated)?;
            return Ok(insert_ordered(
                &updated,
                &nodes,
                &nodes[0],
                "hf",
                &hf,
                MASTER_ORDER,
            ));
        }
    }
    Ok(apply_patches(xml, patches))
}

/// ヘッダーとフッター › ノートと配布資料 › すべてに適用: both masters (made
/// when missing) show what `settings` asks for.
pub(crate) fn set_page_header_footer(bytes: &[u8], settings: &PageHeaderFooter) -> Result<Vec<u8>> {
    let (bytes, notes) = ensure_master(bytes, true)?;
    let (bytes, handout) = ensure_master(&bytes, false)?;
    let mut package = OpcPackage::open_preserving(&bytes)?;
    for part in [notes, handout] {
        let xml = package.required_part_str(&part)?;
        let updated = apply_header_footer(&xml, settings)?;
        package.replace(&part, updated.into_bytes());
    }
    Ok(package.write()?)
}

/// What the notes and handout masters show, read back: the flags and the
/// header, footer and fixed date texts of the notes master (else the
/// handout master).
pub fn page_header_footer(bytes: &[u8]) -> Result<PageHeaderFooter> {
    let package = OpcPackage::open(bytes)?;
    let presentation = presentation_part(&package)?;
    let master = match related(&package, &presentation, "notesMaster")? {
        Some(part) => Some(part),
        None => related(&package, &presentation, "handoutMaster")?,
    };
    let Some(xml) = master.and_then(|part| package.part_str(&part).ok().flatten()) else {
        return Ok(PageHeaderFooter {
            date: true,
            date_text: None,
            header: None,
            page_number: true,
            footer: None,
        });
    };
    let nodes = parse_document_nodes(&xml)?;
    let hf = child(&nodes, &nodes[0], 1, "hf");
    let on = |name: &str| !matches!(hf.and_then(|hf| hf.attr(name)), Some("0" | "false"));
    let text_of = |kind: &str| -> Option<String> {
        let shape = *placeholders(&nodes, kind).first()?;
        let body = child(&nodes, shape, 1, "txBody")?;
        let text: String = nodes
            .iter()
            .filter(|n| {
                n.is(2, "t") && n.outer.start >= body.outer.start && n.outer.end <= body.outer.end
            })
            .map(|n| n.text.as_str())
            .collect();
        Some(text)
    };
    let fixed_date = placeholders(&nodes, "dt").first().and_then(|shape| {
        let has_field = nodes.iter().any(|n| {
            n.is(2, "fld") && n.outer.start >= shape.outer.start && n.outer.end <= shape.outer.end
        });
        (!has_field).then(|| text_of("dt")).flatten()
    });
    Ok(PageHeaderFooter {
        date: on("dt"),
        date_text: fixed_date.filter(|_| on("dt")),
        header: on("hdr").then(|| text_of("hdr").unwrap_or_default()),
        page_number: on("sldNum"),
        footer: on("ftr").then(|| text_of("ftr").unwrap_or_default()),
    })
}

/// Scales every `a:off`/`a:ext` in a master by `(sx, sy)`; a slide image
/// keeps its proportions, centred where its box went.
fn scale_master(xml: &str, sx: f64, sy: f64) -> Result<String> {
    let nodes = parse_document_nodes(xml)?;
    let mut patches = Vec::new();
    for shape in nodes.iter().filter(|n| n.is(1, "sp") || n.is(1, "pic")) {
        let Some(xfrm) =
            child(&nodes, shape, 1, "spPr").and_then(|pr| child(&nodes, pr, 2, "xfrm"))
        else {
            continue;
        };
        let (Some(off), Some(ext)) = (child(&nodes, xfrm, 2, "off"), child(&nodes, xfrm, 2, "ext"))
        else {
            continue;
        };
        let read = |node: &Node, a: &str| node.attr(a).and_then(|v| v.parse::<f64>().ok());
        let (Some(x), Some(y), Some(cx), Some(cy)) = (
            read(off, "x"),
            read(off, "y"),
            read(ext, "cx"),
            read(ext, "cy"),
        ) else {
            continue;
        };
        let keeps_aspect = shape_is_slide_image(&nodes, shape);
        let (nx, ny, ncx, ncy) = if keeps_aspect {
            let (box_w, box_h) = (cx * sx, cy * sy);
            let scale = (box_w / cx).min(box_h / cy);
            let (w, h) = (cx * scale, cy * scale);
            (x * sx + (box_w - w) / 2.0, y * sy + (box_h - h) / 2.0, w, h)
        } else {
            (x * sx, y * sy, cx * sx, cy * sy)
        };
        patches.push((
            off.opening.clone(),
            edit_tag(
                &xml[off.opening.clone()],
                &[
                    ("x", (nx.round() as i64).to_string()),
                    ("y", (ny.round() as i64).to_string()),
                ],
            )?,
        ));
        patches.push((
            ext.opening.clone(),
            edit_tag(
                &xml[ext.opening.clone()],
                &[
                    ("cx", (ncx.round() as i64).to_string()),
                    ("cy", (ncy.round() as i64).to_string()),
                ],
            )?,
        ));
    }
    Ok(apply_patches(xml, patches))
}

fn shape_is_slide_image(nodes: &[Node], shape: &Node) -> bool {
    child(nodes, shape, 1, "nvSpPr")
        .and_then(|nv| child(nodes, nv, 1, "nvPr"))
        .and_then(|nv| child(nodes, nv, 1, "ph"))
        .and_then(|ph| ph.attr("type"))
        == Some("sldImg")
}

/// ノートの向き / 配布資料の向き: turns the notes page (which handouts share)
/// portrait or landscape, moving both masters' placeholders and every
/// notes slide's with it.
pub(crate) fn set_notes_orientation(bytes: &[u8], portrait: bool) -> Result<Vec<u8>> {
    let mut package = OpcPackage::open_preserving(bytes)?;
    let presentation = presentation_part(&package)?;
    let xml = package.required_part_str(&presentation)?;
    let nodes = parse_document_nodes(&xml)?;
    let root = nodes
        .first()
        .filter(|n| n.is(1, "presentation"))
        .ok_or_else(|| xml_error("expected a presentation"))?;
    let ((w, h), _) = sizes(&nodes);
    if (w <= h) == portrait {
        return Ok(bytes.to_vec());
    }
    let (nw, nh) = (h, w);
    let p = prefix(root);
    let size = format!("<{p}notesSz cx=\"{nw}\" cy=\"{nh}\"/>");
    let updated = super::presenting::put_child(
        &xml,
        &nodes,
        root,
        "notesSz",
        &size,
        super::presenting::PRESENTATION_ORDER,
    );
    package.replace(&presentation, updated.into_bytes());
    let (sx, sy) = (nw as f64 / w as f64, nh as f64 / h as f64);
    let mut parts = Vec::new();
    for kind in ["notesMaster", "handoutMaster"] {
        if let Some(part) = related(&package, &presentation, kind)? {
            parts.push(part);
        }
    }
    parts.extend(
        package
            .part_names()
            .iter()
            .filter(|name| name.contains("notesSlides/") && name.ends_with(".xml"))
            .filter(|name| !name.contains("/_rels/"))
            .cloned(),
    );
    for part in parts {
        if let Some(xml) = package.part_str(&part)? {
            let scaled = scale_master(&xml, sx, sy)?;
            package.replace(&part, scaled.into_bytes());
        }
    }
    Ok(package.write()?)
}
