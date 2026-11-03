//! The drawing Office caches beside a diagram's data (`dsp:drawing`):
//! written from laid-out shapes, coloured and styled as the colour scheme
//! and SmartArt style say, and — for a layout jade cannot lay out — its
//! text brought up to date in place.

use std::collections::HashMap;

use crate::colors::{Clr, ColorsDef, Picked, StyleDef, ThemePalette};
use crate::data::{outline, DataModel, A_NS, DGM_NS, DSP_NS, R_NS};
use crate::engine::{self, Drawn, Paint};
use crate::layout_def::Algorithm;
use crate::xml::{self, escape, escape_text, Element, Node as XmlNode};

/// The parts beside a diagram's data that say how it looks; any of them
/// may be absent, and the data's ids name the built-in then.
#[derive(Clone, Copy, Debug, Default)]
pub struct Parts<'a> {
    pub layout: Option<&'a str>,
    pub colors: Option<&'a str>,
    pub style: Option<&'a str>,
}

/// How shapes are painted: the colour scheme, the style, and the theme's
/// colours (for the in-between colours of a span).
pub struct Look<'a> {
    pub colors: ColorsDef,
    pub style: StyleDef,
    pub theme: Option<&'a ThemePalette>,
}

impl<'a> Look<'a> {
    /// The look of `data`: from the parts when given, else the built-ins
    /// its ids name.
    pub fn of(data: &DataModel, parts: Parts<'_>, theme: Option<&'a ThemePalette>) -> Self {
        let colors = parts
            .colors
            .and_then(ColorsDef::parse)
            .unwrap_or_else(|| ColorsDef::builtin(&data.colors_id().unwrap_or_default()));
        let style = parts
            .style
            .and_then(StyleDef::parse)
            .unwrap_or_else(|| StyleDef::builtin(&data.style_id().unwrap_or_default()));
        Self {
            colors,
            style,
            theme,
        }
    }
}

/// Lays `data` out in a `width` × `height` EMU frame and writes its
/// drawing.
pub fn render(
    data_xml: &str,
    parts: Parts<'_>,
    theme: Option<&ThemePalette>,
    width: i64,
    height: i64,
) -> Result<String, String> {
    let data = DataModel::parse(data_xml)?;
    Ok(render_model(&data, parts, theme, width, height))
}

pub fn render_model(
    data: &DataModel,
    parts: Parts<'_>,
    theme: Option<&ThemePalette>,
    width: i64,
    height: i64,
) -> String {
    let layout = data.layout_id().unwrap_or_default();
    let algorithm = parts.layout.and_then(Algorithm::of_layout_def);
    let shapes = engine::lay_out(
        &layout,
        algorithm,
        &data.nodes(),
        width as f64,
        height as f64,
    );
    drawing_xml(&shapes, &Look::of(data, parts, theme))
}

/// Whether jade lays `layout_id` out itself (a built-in it has measured).
pub fn lays_out(layout_id: &str) -> bool {
    crate::catalog::layout(layout_id).is_some()
}

fn color_xml(picked: &Picked, out: &mut String) {
    let Clr { base, mods } = &picked.color;
    out.push_str(&format!("<a:schemeClr val=\"{}\">", escape(base)));
    for (name, value) in mods {
        out.push_str(&format!("<a:{name} val=\"{value}\"/>"));
    }
    for (name, value) in ["hueOff", "satOff", "lumOff", "alphaOff"]
        .iter()
        .zip(picked.offsets)
    {
        if value != 0 {
            out.push_str(&format!("<a:{name} val=\"{value}\"/>"));
        }
    }
    out.push_str("</a:schemeClr>");
}

fn solid(picked: &Picked) -> String {
    let mut out = String::from("<a:solidFill>");
    color_xml(picked, &mut out);
    out.push_str("</a:solidFill>");
    out
}

fn plain(name: &str) -> Picked {
    Picked {
        color: Clr::new(name),
        offsets: [0; 4],
    }
}

const LINE_WIDTHS: [i64; 4] = [0, 6_350, 12_700, 19_050];

fn emu(value: f64) -> i64 {
    value.round() as i64
}

fn model_id(index: usize) -> String {
    format!("{{6C1A2E00-0000-4000-8000-{:012X}}}", index + 1)
}

/// Writes a `dsp:drawing` of `shapes`.
pub fn drawing_xml(shapes: &[Drawn], look: &Look<'_>) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    out.push_str(&format!(
        "<dsp:drawing xmlns:dgm=\"{DGM_NS}\" xmlns:dsp=\"{DSP_NS}\" xmlns:a=\"{A_NS}\" xmlns:r=\"{R_NS}\"><dsp:spTree><dsp:nvGrpSpPr><dsp:cNvPr id=\"0\" name=\"\"/><dsp:cNvGrpSpPr/></dsp:nvGrpSpPr><dsp:grpSpPr/>"
    ));
    let mut used = std::collections::HashSet::new();
    for (index, shape) in shapes.iter().enumerate() {
        // Ids are unique within a drawing: a second shape of one bullet
        // (a body under its header, an arrow after it) gets one of its own.
        let id = shape
            .node
            .clone()
            .filter(|id| used.insert(id.clone()))
            .unwrap_or_else(|| model_id(index));
        shape_xml(shape, &id, look, &mut out);
    }
    out.push_str("</dsp:spTree></dsp:drawing>");
    out
}

fn shape_xml(shape: &Drawn, id: &str, look: &Look<'_>, out: &mut String) {
    out.push_str(&format!(
        "<dsp:sp modelId=\"{}\"><dsp:nvSpPr><dsp:cNvPr id=\"0\" name=\"\"/><dsp:cNvSpPr/></dsp:nvSpPr><dsp:spPr>",
        escape(id)
    ));
    let (x, y, w, h) = match &shape.path {
        Some(path) => {
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            for (px, py) in path {
                x0 = x0.min(*px);
                y0 = y0.min(*py);
                x1 = x1.max(*px);
                y1 = y1.max(*py);
            }
            (x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0))
        }
        None => (shape.x, shape.y, shape.w.max(1.0), shape.h.max(1.0)),
    };
    let rot = emu(shape.rot.rem_euclid(360.0) * 60_000.0);
    if rot != 0 {
        out.push_str(&format!("<a:xfrm rot=\"{rot}\">"));
    } else {
        out.push_str("<a:xfrm>");
    }
    out.push_str(&format!(
        "<a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
        emu(x),
        emu(y),
        emu(w),
        emu(h)
    ));
    if let Some(path) = &shape.path {
        out.push_str(&format!(
            "<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l=\"0\" t=\"0\" r=\"0\" b=\"0\"/><a:pathLst><a:path w=\"{}\" h=\"{}\" fill=\"none\">",
            emu(w),
            emu(h)
        ));
        for (i, (px, py)) in path.iter().enumerate() {
            let tag = if i == 0 { "moveTo" } else { "lnTo" };
            out.push_str(&format!(
                "<a:{tag}><a:pt x=\"{}\" y=\"{}\"/></a:{tag}>",
                emu(px - x),
                emu(py - y)
            ));
        }
        out.push_str("</a:path></a:pathLst></a:custGeom>");
    } else {
        let preset = shape.format.preset.as_deref().unwrap_or(&shape.preset);
        out.push_str(&format!(
            "<a:prstGeom prst=\"{}\"><a:avLst>",
            escape(preset)
        ));
        if shape.format.preset.is_none() {
            for (name, value) in &shape.adjust {
                out.push_str(&format!("<a:gd name=\"{name}\" fmla=\"val {value}\"/>"));
            }
        }
        out.push_str("</a:avLst></a:prstGeom>");
    }

    // Colours and style.
    let (fill, line, text, style) = match &shape.paint {
        Paint::Label {
            label,
            index,
            count,
        } => {
            let colors = look.colors.label(label);
            let style = look.style.label(label);
            let pick = |list: &Option<crate::colors::ClrList>| {
                list.as_ref()
                    .and_then(|list| list.pick(*index, *count, look.theme))
            };
            (
                pick(&colors.fill),
                pick(&colors.line),
                pick(&colors.text),
                style,
            )
        }
        Paint::TextOf {
            label,
            index,
            count,
        } => (
            None,
            None,
            look.colors
                .label(label)
                .text
                .as_ref()
                .and_then(|list| list.pick(*index, *count, look.theme)),
            crate::colors::LabelStyle {
                line: 0,
                fill: 0,
                effect: 0,
                bevel: None,
            },
        ),
        Paint::TextOnly => (
            None,
            None,
            None,
            crate::colors::LabelStyle {
                line: 0,
                fill: 0,
                effect: 0,
                bevel: None,
            },
        ),
    };
    let is_line = shape.path.is_some();
    if let Some(rgb) = &shape.format.fill {
        out.push_str(&format!(
            "<a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>",
            escape(rgb)
        ));
    } else if shape.picture && shape.format.picture.is_some() {
        out.push_str(&format!(
            "<a:blipFill rotWithShape=\"1\"><a:blip r:embed=\"{}\"/><a:srcRect/><a:stretch><a:fillRect/></a:stretch></a:blipFill>",
            escape(shape.format.picture.as_deref().unwrap_or_default())
        ));
    } else if is_line || style.fill == 0 || fill.is_none() {
        out.push_str("<a:noFill/>");
    } else if style.fill == 1 {
        out.push_str(&solid(fill.as_ref().expect("checked")));
    }
    // Fills 2 and 3 are the theme's (gradients): the fill reference says.
    let line_width = LINE_WIDTHS[(style.line as usize).min(3)];
    if let Some(rgb) = &shape.format.line {
        out.push_str(&format!(
            "<a:ln w=\"{}\"><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill></a:ln>",
            line_width.max(12_700),
            escape(rgb)
        ));
    } else if let (Some(color), true) = (&line, style.line > 0) {
        out.push_str(&format!(
            "<a:ln w=\"{line_width}\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\">{}<a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln>",
            solid(color)
        ));
    } else {
        out.push_str("<a:ln><a:noFill/></a:ln>");
    }
    out.push_str("<a:effectLst/>");
    if let Some(bevel) = style.bevel {
        out.push_str(&format!(
            "<a:scene3d><a:camera prst=\"orthographicFront\"/><a:lightRig rig=\"threePt\" dir=\"t\"/></a:scene3d><a:sp3d><a:bevelT w=\"{bevel}\" h=\"{bevel}\"/></a:sp3d>"
        ));
    }
    out.push_str("</dsp:spPr><dsp:style>");
    let reference = |name: &str, idx: u32, color: Option<&Picked>, out: &mut String| {
        out.push_str(&format!("<a:{name} idx=\"{idx}\">"));
        match color {
            Some(color) => color_xml(color, out),
            None => out.push_str("<a:scrgbClr r=\"0\" g=\"0\" b=\"0\"/>"),
        }
        out.push_str(&format!("</a:{name}>"));
    };
    reference("lnRef", style.line, line.as_ref(), out);
    reference("fillRef", style.fill, fill.as_ref(), out);
    reference("effectRef", style.effect, None, out);
    out.push_str("<a:fontRef idx=\"minor\">");
    let text_color = text.unwrap_or_else(|| plain(if fill.is_some() { "lt1" } else { "tx1" }));
    let text_color = if matches!(shape.paint, Paint::TextOnly) {
        plain("tx1")
    } else {
        text_color
    };
    color_xml(&text_color, out);
    out.push_str("</a:fontRef></dsp:style>");

    if !shape.paras.is_empty() {
        text_xml(shape, out);
        let [tx, ty, tw, th] = shape
            .text_box
            .unwrap_or([shape.x, shape.y, shape.w, shape.h]);
        let rot_attr = if shape.text_box.is_none() && rot != 0 {
            format!(" rot=\"{rot}\"")
        } else {
            String::new()
        };
        out.push_str(&format!(
            "<dsp:txXfrm{rot_attr}><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></dsp:txXfrm>",
            emu(tx),
            emu(ty),
            emu(tw.max(1.0)),
            emu(th.max(1.0))
        ));
    }
    out.push_str("</dsp:sp>");
}

fn text_xml(shape: &Drawn, out: &mut String) {
    let size = shape.size_pt.max(1.0);
    let inset = emu(shape.inset * size * 12_700.0);
    let anchor = if shape.top_left { "t" } else { "ctr" };
    out.push_str(&format!(
        "<dsp:txBody><a:bodyPr spcFirstLastPara=\"0\" vert=\"horz\" wrap=\"square\" lIns=\"{inset}\" tIns=\"{inset}\" rIns=\"{inset}\" bIns=\"{inset}\" numCol=\"1\" spcCol=\"1270\" anchor=\"{anchor}\" anchorCtr=\"0\"><a:noAutofit/></a:bodyPr><a:lstStyle/>"
    ));
    let child_size = (size * 0.78).round().max(1.0);
    for para in &shape.paras {
        let level = if shape.bullets {
            para.level + 1
        } else {
            para.level
        };
        let sz = if level > 0 { child_size } else { size };
        let sz = emu(sz * 100.0);
        let tab = emu(sz as f64 * 444.5);
        if level == 0 {
            let algn = if shape.top_left { "l" } else { "ctr" };
            out.push_str(&format!(
                "<a:p><a:pPr marL=\"0\" lvl=\"0\" indent=\"0\" algn=\"{algn}\" defTabSz=\"{tab}\"><a:lnSpc><a:spcPct val=\"90000\"/></a:lnSpc><a:spcBef><a:spcPct val=\"0\"/></a:spcBef><a:spcAft><a:spcPct val=\"35000\"/></a:spcAft><a:buNone/></a:pPr>"
            ));
        } else {
            out.push_str(&format!(
                "<a:p><a:pPr marL=\"{}\" lvl=\"{level}\" indent=\"-171450\" algn=\"l\" defTabSz=\"{tab}\"><a:lnSpc><a:spcPct val=\"90000\"/></a:lnSpc><a:spcBef><a:spcPct val=\"0\"/></a:spcBef><a:spcAft><a:spcPct val=\"15000\"/></a:spcAft><a:buChar char=\"&#8226;\"/></a:pPr>",
                171_450 * level
            ));
        }
        let color = shape
            .format
            .text
            .as_ref()
            .map(|rgb| {
                format!(
                    "<a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>",
                    escape(rgb)
                )
            })
            .unwrap_or_default();
        if para.text.is_empty() {
            out.push_str(&format!(
                "<a:endParaRPr lang=\"ja-JP\" altLang=\"en-US\" sz=\"{sz}\" kern=\"1200\"/></a:p>"
            ));
        } else {
            out.push_str(&format!(
                "<a:r><a:rPr lang=\"ja-JP\" altLang=\"en-US\" sz=\"{sz}\" kern=\"1200\">{color}</a:rPr><a:t>{}</a:t></a:r></a:p>",
                escape_text(&para.text)
            ));
        }
    }
    out.push_str("</dsp:txBody>");
}

/// Brings the text of a drawing (one PowerPoint laid out, of a layout jade
/// does not lay out itself) up to date with `data`: each shape shows the
/// bullets its presentation point is linked to, keeping the paragraph and
/// run formatting it had.
pub fn update_text(drawing: &str, data: &DataModel) -> Result<String, String> {
    let mut root = xml::parse(drawing)?;
    let shown = data.shown_in();
    let nodes = data.nodes();
    let flat = outline(&nodes);
    let by_id: HashMap<&str, (usize, &str)> = flat
        .iter()
        .map(|(level, node)| (node.id.as_str(), (*level, node.text.as_str())))
        .collect();
    let Some(tree) = root.child_mut("spTree") else {
        return Ok(drawing.to_string());
    };
    for sp in tree.elements_mut().filter(|e| e.local() == "sp") {
        let Some(id) = sp.attr("modelId").map(str::to_string) else {
            continue;
        };
        let ids: Vec<String> = match shown.get(&id) {
            Some(ids) => ids.clone(),
            None if by_id.contains_key(id.as_str()) => vec![id.clone()],
            None => continue,
        };
        let lines: Vec<(usize, String)> = {
            let base = ids
                .iter()
                .filter_map(|id| by_id.get(id.as_str()).map(|(level, _)| *level))
                .min()
                .unwrap_or(0);
            ids.iter()
                .filter_map(|id| by_id.get(id.as_str()))
                .flat_map(|(level, text)| {
                    text.split('\n')
                        .map(move |line| (level - base, line.to_string()))
                })
                .collect()
        };
        let Some(body) = sp.child_mut("txBody") else {
            continue;
        };
        let old: Vec<Element> = body
            .elements()
            .filter(|e| e.local() == "p")
            .cloned()
            .collect();
        let template = |level: usize| -> Option<&Element> {
            old.iter()
                .find(|p| {
                    p.child("pPr")
                        .and_then(|ppr| ppr.attr("lvl"))
                        .and_then(|l| l.parse::<usize>().ok())
                        .unwrap_or(0)
                        == level
                })
                .or_else(|| old.last())
        };
        let mut paragraphs = Vec::new();
        for (level, text) in &lines {
            let mut p = Element::new("a:p");
            let source = template(*level);
            if let Some(ppr) = source.and_then(|p| p.child("pPr")) {
                p.children.push(XmlNode::Element(ppr.clone()));
            }
            let rpr = source.and_then(|p| {
                p.descendant("rPr")
                    .cloned()
                    .or_else(|| p.child("endParaRPr").cloned())
            });
            if text.is_empty() {
                let mut end = rpr.unwrap_or_else(|| Element::new("a:endParaRPr"));
                end.name = "a:endParaRPr".into();
                p.children.push(XmlNode::Element(end));
            } else {
                let mut rpr = rpr.unwrap_or_else(|| Element::new("a:rPr"));
                rpr.name = "a:rPr".into();
                let mut t = Element::new("a:t");
                t.children.push(XmlNode::Text(text.clone()));
                p.children.push(XmlNode::Element(
                    Element::new("a:r").with_child(rpr).with_child(t),
                ));
            }
            paragraphs.push(p);
        }
        body.children
            .retain(|node| !matches!(node, XmlNode::Element(e) if e.local() == "p"));
        body.children
            .extend(paragraphs.into_iter().map(XmlNode::Element));
    }
    Ok(root.to_document())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Node;

    fn sample() -> DataModel {
        let mut data = DataModel::new("process1", "accent1_2", "simple1");
        data.set_nodes(&[
            Node::with("A", vec![Node::new("A1")]),
            Node::new("B"),
            Node::new("C"),
        ]);
        data
    }

    #[test]
    fn writes_a_drawing_with_every_bullet_and_the_scheme_colours() {
        let data = sample();
        let xml = render_model(&data, Parts::default(), None, 8_128_000, 5_418_667);
        let root = xml::parse(&xml).unwrap();
        let mut shapes = Vec::new();
        root.descendants("sp", &mut shapes);
        // Three boxes and two arrows.
        assert_eq!(shapes.len(), 5);
        assert!(xml.contains("<a:t>A1</a:t>"));
        assert!(xml.contains("prst=\"rightArrow\""));
        assert!(xml.contains("<a:schemeClr val=\"accent1\">"));
        assert!(xml.contains("<a:buChar"));
    }

    #[test]
    fn a_span_writes_the_offsets_between_its_ends() {
        let mut data = sample();
        data.set_colors_id("colorful2");
        let mut theme = ThemePalette::new();
        theme.insert("accent2".into(), (0xED, 0x7D, 0x31));
        theme.insert("accent3".into(), (0xA5, 0xA5, 0xA5));
        let xml = render_model(&data, Parts::default(), Some(&theme), 8_128_000, 5_418_667);
        assert!(
            xml.contains("<a:hueOff") || xml.contains("<a:satOff"),
            "{xml}"
        );
    }

    #[test]
    fn updates_the_text_of_a_drawing_in_place() {
        let mut data = sample();
        let drawing = render_model(&data, Parts::default(), None, 8_128_000, 5_418_667);
        let mut nodes = data.nodes();
        nodes[1].text = "Beta".into();
        data.set_nodes(&nodes);
        let updated = update_text(&drawing, &data).unwrap();
        assert!(updated.contains("<a:t>Beta</a:t>"));
        assert!(!updated.contains("<a:t>B</a:t>"));
        assert!(updated.contains("sz=\""));
    }
}
