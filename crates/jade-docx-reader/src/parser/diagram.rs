//! SmartArt: Word's cached diagram drawing (`word/diagrams/drawingN.xml`,
//! root `dsp:drawing`). Word writes this pre-rendered DrawingML shape tree
//! next to the diagram data so consumers can draw a diagram without running
//! its layout definition. It uses the same `spPr`/`txBody` vocabulary as a
//! WordprocessingGroup, so it is parsed into a [`RunShapeGroup`] and drawn by
//! the grouped-shape renderer. The package parts themselves are never
//! rewritten.

use super::*;

/// Text body facts of one `dsp:sp`, in document order.
#[derive(Default)]
struct DiagramShapeText {
    text: String,
    style: GroupShapeTextStyle,
    has_style: bool,
    font_ref_color: Option<String>,
    font_ref_theme_color: Option<String>,
    font_ref_transforms: Vec<ThemeColorTransform>,
    insets: Option<TextBoxInsets>,
    vertical_alignment: TextBoxVerticalAlignment,
    shape_box: Option<(f64, f64, f64, f64)>,
    text_box: Option<(f64, f64, f64, f64)>,
}

fn emu_attribute(element: &BytesStart<'_>, name: &[u8]) -> Option<f64> {
    element.attributes().flatten().find_map(|attribute| {
        (attribute.key.as_ref() == name)
            .then(|| attribute.unescape_value().ok()?.trim().parse::<f64>().ok())
            .flatten()
    })
}

fn string_attribute(element: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    element.attributes().flatten().find_map(|attribute| {
        (attribute.key.as_ref() == name)
            .then(|| {
                attribute
                    .unescape_value()
                    .ok()
                    .map(|value| value.into_owned())
            })
            .flatten()
    })
}

fn drawingml_bool(element: &BytesStart<'_>, name: &[u8]) -> Option<bool> {
    string_attribute(element, name).map(|value| matches!(value.trim(), "1" | "true" | "on"))
}

/// The WordprocessingGroup element name a `dsp:` element is read as, `None`
/// for subtrees the geometry pass skips (text and style are read
/// separately), or the original name.
fn group_vocabulary(name: &[u8]) -> Option<Option<&'static str>> {
    match name {
        b"dsp:grpSp" => Some(Some("wpg:grpSp")),
        b"dsp:grpSpPr" => Some(Some("wpg:grpSpPr")),
        b"dsp:sp" => Some(Some("wps:wsp")),
        b"dsp:spPr" => Some(Some("wps:spPr")),
        b"dsp:txBody" | b"dsp:txXfrm" | b"dsp:style" => Some(None),
        _ => None,
    }
}

impl DocumentParser {
    /// Parses a SmartArt `dsp:drawing` part into a shape group whose child
    /// coordinates are the drawing's EMUs. `frame_extent_emu` is the graphic
    /// frame's `wp:extent`, which is the coordinate space Word caches the
    /// drawing in. Returns `None` when the part holds no shapes.
    pub fn parse_diagram_drawing(
        &self,
        xml: &str,
        frame_extent_emu: (f64, f64),
    ) -> Result<Option<RunShapeGroup>, Box<dyn std::error::Error>> {
        let renamed = Self::diagram_drawing_as_group_xml(xml)?;
        let mut group = self.parse_shape_group(&renamed)?;
        if group.children.is_empty() && group.groups.is_empty() {
            return Ok(None);
        }
        if group.child_extent_width <= 0.0 || group.child_extent_height <= 0.0 {
            group.child_offset_x = 0.0;
            group.child_offset_y = 0.0;
            group.child_extent_width = frame_extent_emu.0;
            group.child_extent_height = frame_extent_emu.1;
        }
        let mut texts = Self::diagram_shape_texts(xml)?.into_iter();
        Self::assign_diagram_texts(&mut group, &mut texts);
        Ok(Some(group))
    }

    /// Rewrites the drawing's `dsp:` shape tree into the WordprocessingGroup
    /// vocabulary `parse_shape_group` reads, dropping text and style
    /// subtrees.
    fn diagram_drawing_as_group_xml(xml: &str) -> Result<String, Box<dyn std::error::Error>> {
        let mut reader = Reader::from_str(xml);
        let mut writer = Writer::new(Cursor::new(Vec::new()));
        let mut buf = Vec::new();
        let mut skip_depth = 0usize;
        let mut in_tree = false;
        loop {
            let event = reader.read_event_into(&mut buf)?;
            match event {
                Event::Eof => break,
                Event::Start(ref element) => {
                    if skip_depth > 0 {
                        skip_depth += 1;
                    } else if element.name().as_ref() == b"dsp:spTree" {
                        in_tree = true;
                    } else if in_tree {
                        match group_vocabulary(element.name().as_ref()) {
                            Some(None) => skip_depth = 1,
                            Some(Some(name)) => {
                                let mut renamed = BytesStart::new(name);
                                renamed.extend_attributes(element.attributes().flatten());
                                writer.write_event(Event::Start(renamed))?;
                            }
                            None => writer.write_event(Event::Start(element.clone()))?,
                        }
                    }
                }
                Event::End(ref element) => {
                    if skip_depth > 0 {
                        skip_depth -= 1;
                    } else if element.name().as_ref() == b"dsp:spTree" {
                        in_tree = false;
                    } else if in_tree {
                        match group_vocabulary(element.name().as_ref()) {
                            Some(Some(name)) => {
                                writer.write_event(Event::End(
                                    quick_xml::events::BytesEnd::new(name),
                                ))?;
                            }
                            Some(None) => {}
                            None => writer.write_event(Event::End(element.clone()))?,
                        }
                    }
                }
                Event::Empty(ref element) => {
                    if skip_depth == 0 && in_tree {
                        match group_vocabulary(element.name().as_ref()) {
                            Some(None) => {}
                            Some(Some(name)) => {
                                let mut renamed = BytesStart::new(name);
                                renamed.extend_attributes(element.attributes().flatten());
                                writer.write_event(Event::Empty(renamed))?;
                            }
                            None => writer.write_event(Event::Empty(element.clone()))?,
                        }
                    }
                }
                Event::Text(ref text) if skip_depth == 0 && in_tree => {
                    writer.write_event(Event::Text(text.clone()))?;
                }
                Event::GeneralRef(ref reference) if skip_depth == 0 && in_tree => {
                    writer.write_event(Event::GeneralRef(reference.clone()))?;
                }
                _ => {}
            }
            buf.clear();
        }
        Ok(String::from_utf8(writer.into_inner().into_inner())?)
    }

    /// Reads each `dsp:sp`'s text body, text frame and font style, in
    /// document order (the order `parse_shape_group` assigns `z_order`).
    fn diagram_shape_texts(xml: &str) -> Result<Vec<DiagramShapeText>, Box<dyn std::error::Error>> {
        let mut reader = Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut shapes = Vec::new();
        let mut current: Option<DiagramShapeText> = None;
        let mut in_sp_pr = false;
        let mut in_sp_xfrm = false;
        let mut in_tx_xfrm = false;
        let mut in_font_ref = false;
        let mut in_run_properties = false;
        let mut run_properties_seen = false;
        let mut in_run_fill = false;
        let mut in_text = false;
        let mut paragraph_count = 0usize;
        // Where color transforms go: `Some(true)` the run fill, `Some(false)`
        // the style's `a:fontRef`.
        let mut transform_target: Option<bool> = None;

        loop {
            let event = reader.read_event_into(&mut buf)?;
            let (element, is_start) = match &event {
                Event::Start(element) => (Some(element.clone()), true),
                Event::Empty(element) => (Some(element.clone()), false),
                Event::End(element) => {
                    match element.name().as_ref() {
                        b"dsp:sp" => {
                            if let Some(shape) = current.take() {
                                shapes.push(shape);
                            }
                        }
                        b"dsp:spPr" => in_sp_pr = false,
                        b"a:xfrm" => in_sp_xfrm = false,
                        b"dsp:txXfrm" => in_tx_xfrm = false,
                        b"a:fontRef" => {
                            in_font_ref = false;
                            transform_target = None;
                        }
                        b"a:rPr" | b"a:endParaRPr" => {
                            in_run_properties = false;
                            in_run_fill = false;
                            transform_target = None;
                        }
                        b"a:solidFill" => in_run_fill = false,
                        b"a:t" => in_text = false,
                        _ => {}
                    }
                    buf.clear();
                    continue;
                }
                Event::Text(text) => {
                    if in_text {
                        if let (Some(shape), Some(value)) =
                            (current.as_mut(), text_event_to_string(text))
                        {
                            shape.text.push_str(&value);
                        }
                    }
                    buf.clear();
                    continue;
                }
                Event::GeneralRef(reference) => {
                    if in_text {
                        if let (Some(shape), Some(value)) =
                            (current.as_mut(), reference_event_to_string(reference))
                        {
                            shape.text.push_str(&value);
                        }
                    }
                    buf.clear();
                    continue;
                }
                Event::Eof => break,
                _ => {
                    buf.clear();
                    continue;
                }
            };
            let Some(element) = element else {
                buf.clear();
                continue;
            };
            let name = element.name();
            match name.as_ref() {
                b"dsp:sp" if is_start => {
                    current = Some(DiagramShapeText::default());
                    paragraph_count = 0;
                    run_properties_seen = false;
                }
                b"dsp:spPr" if is_start => in_sp_pr = true,
                b"a:xfrm" if in_sp_pr && is_start => in_sp_xfrm = true,
                b"dsp:txXfrm" if is_start => in_tx_xfrm = true,
                b"a:off" | b"a:ext" if in_sp_xfrm || in_tx_xfrm => {
                    if let Some(shape) = current.as_mut() {
                        let target = if in_tx_xfrm {
                            &mut shape.text_box
                        } else {
                            &mut shape.shape_box
                        };
                        let rect = target.get_or_insert((0.0, 0.0, 0.0, 0.0));
                        if name.as_ref() == b"a:off" {
                            rect.0 = emu_attribute(&element, b"x").unwrap_or(0.0);
                            rect.1 = emu_attribute(&element, b"y").unwrap_or(0.0);
                        } else {
                            rect.2 = emu_attribute(&element, b"cx").unwrap_or(0.0);
                            rect.3 = emu_attribute(&element, b"cy").unwrap_or(0.0);
                        }
                    }
                }
                b"a:bodyPr" => {
                    if let Some(shape) = current.as_mut() {
                        let mut insets = TextBoxInsets::default();
                        Self::populate_text_box_body_properties(
                            &mut insets,
                            &mut shape.vertical_alignment,
                            &element,
                        );
                        shape.insets = Some(insets);
                    }
                }
                b"a:fontRef" if is_start => {
                    in_font_ref = true;
                    transform_target = Some(false);
                }
                b"a:p" if is_start => {
                    if let Some(shape) = current.as_mut() {
                        if paragraph_count > 0 {
                            shape.text.push('\n');
                        }
                    }
                    paragraph_count += 1;
                }
                b"a:pPr" => {
                    if let Some(shape) = current.as_mut() {
                        if shape.style.alignment.is_none() {
                            shape.style.alignment =
                                string_attribute(&element, b"algn").map(|value| {
                                    match value.as_str() {
                                        "ctr" => "center".to_string(),
                                        "r" => "right".to_string(),
                                        "just" | "dist" => "both".to_string(),
                                        _ => "left".to_string(),
                                    }
                                });
                        }
                    }
                }
                b"a:buChar" => {
                    if let (Some(shape), Some(bullet)) =
                        (current.as_mut(), string_attribute(&element, b"char"))
                    {
                        shape.text.push_str(&bullet);
                        shape.text.push(' ');
                    }
                }
                b"a:br" => {
                    if let Some(shape) = current.as_mut() {
                        shape.text.push('\n');
                    }
                }
                b"a:t" if is_start => in_text = true,
                b"a:rPr" | b"a:endParaRPr" => {
                    // The first run's properties style the whole body; an
                    // end-of-paragraph mark only fills in a missing size.
                    let is_run = name.as_ref() == b"a:rPr";
                    if let Some(shape) = current.as_mut() {
                        if is_run && !run_properties_seen {
                            run_properties_seen = true;
                            shape.has_style = true;
                            shape.style.font_size_pt =
                                emu_attribute(&element, b"sz").map(|size| (size / 100.0) as f32);
                            shape.style.bold = drawingml_bool(&element, b"b").unwrap_or(false);
                            shape.style.italic = drawingml_bool(&element, b"i").unwrap_or(false);
                            in_run_properties = is_start;
                        } else if shape.style.font_size_pt.is_none() {
                            shape.style.font_size_pt =
                                emu_attribute(&element, b"sz").map(|size| (size / 100.0) as f32);
                        }
                    }
                }
                b"a:solidFill" if in_run_properties && is_start => {
                    in_run_fill = true;
                    transform_target = Some(true);
                }
                b"a:srgbClr" | b"a:scrgbClr" | b"a:hslClr" | b"a:sysClr" | b"a:prstClr"
                    if in_run_fill || in_font_ref =>
                {
                    if let (Some(shape), Some(color)) = (
                        current.as_mut(),
                        crate::color::parse_explicit_color(&element),
                    ) {
                        if in_run_fill {
                            shape.style.color = Some(color);
                            shape.style.theme_color = None;
                        } else {
                            shape.font_ref_color = Some(color);
                        }
                    }
                }
                b"a:schemeClr" if in_run_fill || in_font_ref => {
                    if let (Some(shape), Some(value)) =
                        (current.as_mut(), Self::drawingml_color_value(&element))
                    {
                        if in_run_fill {
                            shape.style.theme_color = Some(value);
                            shape.style.color = None;
                        } else {
                            shape.font_ref_theme_color = Some(value);
                        }
                    }
                }
                b"a:lumMod" | b"a:lumOff" | b"a:tint" | b"a:shade" => {
                    if let (Some(shape), Some(transform), Some(run)) = (
                        current.as_mut(),
                        Self::drawingml_theme_color_transform(&element),
                        transform_target,
                    ) {
                        if run {
                            shape.style.theme_transforms.push(transform);
                        } else {
                            shape.font_ref_transforms.push(transform);
                        }
                    }
                }
                _ => {}
            }
            buf.clear();
        }

        for shape in &mut shapes {
            if shape.style.color.is_none() && shape.style.theme_color.is_none() {
                shape.style.color = shape.font_ref_color.take();
                shape.style.theme_color = shape.font_ref_theme_color.take();
                shape.style.theme_transforms = std::mem::take(&mut shape.font_ref_transforms);
            }
            // Explicit colors carry their transforms already resolved.
            if let Some(color) = shape.style.color.as_mut() {
                if !shape.style.theme_transforms.is_empty() && shape.style.theme_color.is_none() {
                    if let Some(resolved) =
                        crate::color::apply_color_transforms(color, &shape.style.theme_transforms)
                    {
                        *color = resolved;
                    }
                    shape.style.theme_transforms.clear();
                }
            }
            shape.has_style |= shape.style.color.is_some() || shape.style.theme_color.is_some();
        }
        Ok(shapes)
    }

    /// Hands the text of each drawing shape to the group child created for
    /// it. Children and nested groups share one `z_order` sequence that
    /// follows document order, so a depth-first walk in that order visits
    /// the shapes in the order they were read.
    fn assign_diagram_texts(
        group: &mut RunShapeGroup,
        texts: &mut impl Iterator<Item = DiagramShapeText>,
    ) {
        enum Item {
            Shape(usize),
            Group(usize),
        }
        let mut items = group
            .children
            .iter()
            .enumerate()
            .map(|(index, shape)| (shape.z_order, Item::Shape(index)))
            .chain(
                group
                    .groups
                    .iter()
                    .enumerate()
                    .map(|(index, nested)| (nested.z_order, Item::Group(index))),
            )
            .collect::<Vec<_>>();
        items.sort_by_key(|(z_order, _)| *z_order);
        for (_, item) in items {
            match item {
                Item::Group(index) => Self::assign_diagram_texts(&mut group.groups[index], texts),
                Item::Shape(index) => {
                    let Some(text) = texts.next() else {
                        return;
                    };
                    let shape = &mut group.children[index];
                    shape.text = text.text.trim_end_matches('\n').to_string();
                    shape.vertical_alignment = text.vertical_alignment;
                    let mut insets = text.insets.unwrap_or_default();
                    // `dsp:txXfrm` is the text rectangle Word laid the text
                    // out in; fold its offset from the shape into the insets.
                    if let (Some(shape_box), Some(text_box)) = (text.shape_box, text.text_box) {
                        let px = |emu: f64| (emu / 9_525.0).max(0.0);
                        insets.left_px += px(text_box.0 - shape_box.0);
                        insets.top_px += px(text_box.1 - shape_box.1);
                        insets.right_px +=
                            px((shape_box.0 + shape_box.2) - (text_box.0 + text_box.2));
                        insets.bottom_px +=
                            px((shape_box.1 + shape_box.3) - (text_box.1 + text_box.3));
                    }
                    shape.insets = insets;
                    if text.has_style {
                        shape.text_style = Some(text.style);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRAWING: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<dsp:drawing xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
  <dsp:spTree>
    <dsp:nvGrpSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvGrpSpPr/></dsp:nvGrpSpPr>
    <dsp:grpSpPr/>
    <dsp:sp modelId="{1}">
      <dsp:nvSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvSpPr/></dsp:nvSpPr>
      <dsp:spPr>
        <a:xfrm><a:off x="0" y="0"/><a:ext cx="1828800" cy="914400"/></a:xfrm>
        <a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom>
        <a:solidFill><a:srgbClr val="4472C4"/></a:solidFill>
        <a:ln w="12700"><a:solidFill><a:srgbClr val="FFFFFF"/></a:solidFill></a:ln>
      </dsp:spPr>
      <dsp:style>
        <a:lnRef idx="2"><a:scrgbClr r="0" g="0" b="0"/></a:lnRef>
        <a:fillRef idx="1"><a:scrgbClr r="0" g="0" b="0"/></a:fillRef>
        <a:effectRef idx="0"><a:scrgbClr r="0" g="0" b="0"/></a:effectRef>
        <a:fontRef idx="minor"><a:schemeClr val="lt1"/></a:fontRef>
      </dsp:style>
      <dsp:txBody>
        <a:bodyPr spcFirstLastPara="0" vert="horz" wrap="square" lIns="76200" tIns="76200" rIns="76200" bIns="76200" numCol="1" spcCol="1270" anchor="ctr" anchorCtr="0"><a:noAutofit/></a:bodyPr>
        <a:lstStyle/>
        <a:p><a:pPr marL="0" lvl="0" indent="0" algn="ctr"/><a:r><a:rPr lang="en-US" sz="2000" kern="1200" b="1"/><a:t>Plan</a:t></a:r></a:p>
      </dsp:txBody>
      <dsp:txXfrm><a:off x="95250" y="0"/><a:ext cx="1638300" cy="914400"/></dsp:txXfrm>
    </dsp:sp>
    <dsp:sp modelId="{2}">
      <dsp:nvSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvSpPr/></dsp:nvSpPr>
      <dsp:spPr>
        <a:xfrm rot="5400000"><a:off x="2057400" y="228600"/><a:ext cx="457200" cy="457200"/></a:xfrm>
        <a:prstGeom prst="rightArrow"><a:avLst/></a:prstGeom>
        <a:solidFill><a:schemeClr val="accent1"><a:tint val="60000"/></a:schemeClr></a:solidFill>
      </dsp:spPr>
      <dsp:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang="en-US"/></a:p></dsp:txBody>
    </dsp:sp>
    <dsp:sp modelId="{3}">
      <dsp:nvSpPr><dsp:cNvPr id="0" name=""/><dsp:cNvSpPr/></dsp:nvSpPr>
      <dsp:spPr>
        <a:xfrm><a:off x="2743200" y="0"/><a:ext cx="1828800" cy="914400"/></a:xfrm>
        <a:prstGeom prst="rect"><a:avLst/></a:prstGeom>
      </dsp:spPr>
      <dsp:txBody>
        <a:bodyPr anchor="t"/>
        <a:p><a:pPr algn="l"><a:buChar char="&#8226;"/></a:pPr><a:r><a:rPr sz="1400" i="1"><a:solidFill><a:srgbClr val="C00000"/></a:solidFill></a:rPr><a:t>Do</a:t></a:r></a:p>
        <a:p><a:r><a:rPr sz="1400"/><a:t>Check &amp; act</a:t></a:r></a:p>
      </dsp:txBody>
    </dsp:sp>
  </dsp:spTree>
</dsp:drawing>"#;

    #[test]
    fn parses_cached_smartart_drawing_into_a_styled_shape_group() {
        let group = DocumentParser::new()
            .parse_diagram_drawing(DRAWING, (4_572_000.0, 914_400.0))
            .expect("parse")
            .expect("shapes");
        assert_eq!(group.child_extent_width, 4_572_000.0);
        assert_eq!(group.child_extent_height, 914_400.0);
        assert_eq!(group.children.len(), 3);

        let first = &group.children[0];
        assert_eq!(first.shape, TextBoxShape::RoundedRectangle);
        assert_eq!(first.fill_color.as_deref(), Some("#4472C4"));
        assert_eq!(first.width, 1_828_800.0);
        assert_eq!(first.text, "Plan");
        assert_eq!(first.vertical_alignment, TextBoxVerticalAlignment::Center);
        // bodyPr inset (8px) plus the txXfrm offset (10px) on each side.
        assert!((first.insets.left_px - 18.0).abs() < 1e-9);
        assert!((first.insets.right_px - 18.0).abs() < 1e-9);
        assert!((first.insets.top_px - 8.0).abs() < 1e-9);
        let style = first.text_style.as_ref().expect("text style");
        assert_eq!(style.font_size_pt, Some(20.0));
        assert!(style.bold);
        assert_eq!(style.alignment.as_deref(), Some("center"));
        // No run fill: the shape style's fontRef color applies.
        assert_eq!(style.theme_color.as_deref(), Some("lt1"));

        let arrow = &group.children[1];
        assert!((arrow.rotation_deg - 90.0).abs() < 1e-9);
        assert_eq!(arrow.fill_theme_color.as_deref(), Some("accent1"));
        assert!(arrow.text.is_empty());

        let list = &group.children[2];
        assert_eq!(list.text, "\u{2022} Do\nCheck & act");
        let style = list.text_style.as_ref().expect("list style");
        assert_eq!(style.color.as_deref(), Some("#C00000"));
        assert!(style.italic);
        assert_eq!(style.font_size_pt, Some(14.0));
        assert_eq!(style.alignment.as_deref(), Some("left"));
    }

    #[test]
    fn empty_drawing_yields_no_group() {
        let xml = r#"<dsp:drawing xmlns:dsp="http://schemas.microsoft.com/office/drawing/2008/diagram"><dsp:spTree/></dsp:drawing>"#;
        assert!(DocumentParser::new()
            .parse_diagram_drawing(xml, (1.0, 1.0))
            .expect("parse")
            .is_none());
    }
}
