use super::*;
use crate::commands::set_run_format::apply_to_properties;
use crate::commands::set_run_format::run_format_of;
use jade_docx_headless_api::HeadlessRunFormat;

impl WordDocument {
    /// Read a run's properties through the ordinary reader, without parsing or
    /// reserializing its text, field results, or other inline objects.
    fn hf_direct_run_properties(run: &XmlElement) -> Result<RunProperties, String> {
        let mut probe = DocxXmlDocument::parse(
            r#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:p><w:r><w:t>x</w:t></w:r></w:p></w:hdr>"#,
        ).map_err(|error| error.to_string())?;
        let (_, runs) = Self::hf_paragraph_run_ids(&probe, 0)?;
        if let Some(rpr) = Self::run_properties_element(run) {
            probe
                .insert_node(runs[0], 0, XmlNode::Element(rpr.clone()))
                .map_err(|error| error.to_string())?;
        }
        let xml = probe.to_string().map_err(|error| error.to_string())?;
        let parsed = DocumentParser::new()
            .parse_document_xml(&xml)
            .map_err(|error| error.to_string())?;
        let mut properties = parsed
            .body
            .as_ref()
            .and_then(|body| body.elements.first())
            .and_then(BodyElement::as_paragraph)
            .and_then(|paragraph| paragraph.runs.first())
            .and_then(|run| run.properties.clone())
            .unwrap_or_default();
        properties.capture_direct_font_properties();
        properties.preserved_rpr = PreservedRunProps(
            Self::run_properties_element(run)
                .into_iter()
                .flat_map(|rpr| &rpr.children)
                .filter(|node| {
                    matches!(node, XmlNode::Element(child)
                if !Self::RUN_PROP_HANDLED_TAGS.contains(&child.name.as_str()))
                })
                .cloned()
                .collect(),
        );
        Ok(properties)
    }

    /// The Font dialog follows the active story and the selected run, including
    /// empty formatting runs. Field paragraphs retain their whole-line fallback.
    // The document core shares this file without the visual query adapter.
    #[allow(dead_code)]
    pub(crate) fn hf_run_format_at(
        &self,
        role: HeaderFooterRole,
        variant: HeaderFooterVariant,
        paragraph: usize,
        offset: usize,
    ) -> HeadlessRunFormat {
        let result = (|| {
            let reference = self.active_hf_reference(role, variant)?;
            let path = self.hf_part_path_for_reference(&reference)?;
            let xml = DocxXmlDocument::parse(&self.read_xml_part_content(&path)?).ok()?;
            let (paragraph_id, ids) = Self::hf_paragraph_run_ids(&xml, paragraph).ok()?;
            let id = if let Some(runs) = Self::hf_text_runs(&xml, paragraph_id) {
                let length = runs.last().map_or(0, |run| run.end);
                let target = offset.min(length.saturating_sub(1));
                runs.iter().find(|run| target < run.end).or(runs.last())?.id
            } else {
                *ids.iter()
                    .find(|id| {
                        xml.element(**id).is_some_and(|run| {
                            run.children.iter().any(|node| {
                                matches!(node, XmlNode::Element(child)
                        if child.name == "w:t")
                            })
                        })
                    })
                    .or(ids.first())?
            };
            let properties = Self::hf_direct_run_properties(xml.element(id)?).ok()?;
            Some(run_format_of(&properties))
        })();
        result.unwrap_or_default()
    }

    /// Patch only the rPr children the dialog mentions. The shared command
    /// normalizes values and dependent properties; original text and unrelated
    /// XML never pass through a model writer.
    pub(super) fn patch_hf_run_properties(
        xml: &mut DocxXmlDocument,
        run_id: XmlNodeId,
        patch: &HeadlessRunFormat,
        layout_id: i64,
    ) -> Result<(), String> {
        let run = xml.element(run_id).ok_or("missing story run")?;
        let mut properties = Self::hf_direct_run_properties(run)?;
        let existing = Self::run_properties_element(run).cloned();
        apply_to_properties(&mut properties, patch, layout_id);
        let generated = Self::sync_run_properties_element(xml, Some(&properties), None)
            .ok_or("missing generated run properties")?;
        let mut rpr = existing.unwrap_or_else(|| xml.create_element("w:rPr", vec![], vec![]));
        let affected = |name: &str| hf_patch_affects(patch, name);
        // rFonts carries independent scripts and optional vendor attributes.
        // Changing one script must not reconstruct the others.
        let fonts = rpr.children.iter().find_map(|node| match node {
            XmlNode::Element(child) if child.name == "w:rFonts" => Some(child.clone()),
            _ => None,
        });
        let mut generated = generated;
        if let Some(mut fonts) = fonts {
            if let Some(new_fonts) = generated.children.iter_mut().find_map(|node| match node {
                XmlNode::Element(child) if child.name == "w:rFonts" => Some(child),
                _ => None,
            }) {
                let selected = |name: &str| match name {
                    "w:ascii" | "w:hAnsi" | "w:asciiTheme" | "w:hAnsiTheme" => {
                        nonempty(&patch.latin_font)
                    }
                    "w:eastAsia" | "w:eastAsiaTheme" => nonempty(&patch.east_asia_font),
                    "w:cs" | "w:cstheme" | "w:csTheme" => nonempty(&patch.complex_font),
                    _ => false,
                };
                fonts
                    .attributes
                    .retain(|attribute| !selected(&attribute.name));
                fonts.attributes.extend(
                    new_fonts
                        .attributes
                        .iter()
                        .filter(|attribute| selected(&attribute.name))
                        .cloned(),
                );
                *new_fonts = fonts;
            }
        }
        rpr.children
            .retain(|node| !matches!(node, XmlNode::Element(child) if affected(&child.name)));
        rpr.children.extend(
            generated
                .children
                .into_iter()
                .filter(|node| matches!(node, XmlNode::Element(child) if affected(&child.name))),
        );
        if rpr
            .children
            .iter()
            .any(|node| matches!(node, XmlNode::Element(child) if child.name.starts_with("w14:")))
        {
            let root_id = xml.root().id;
            let root = xml.element_mut(root_id).ok_or("missing story root")?;
            if !root
                .attributes
                .iter()
                .any(|attribute| attribute.name == "xmlns:w14")
            {
                root.attributes.push(XmlAttribute {
                    name: "xmlns:w14".into(),
                    value: W14_NAMESPACE.into(),
                });
            }
        }
        let run = xml.element_mut(run_id).ok_or("missing story run")?;
        run.children
            .retain(|node| !matches!(node, XmlNode::Element(child) if child.name == "w:rPr"));
        run.children.insert(0, XmlNode::Element(rpr));
        Ok(())
    }
}

fn nonempty(value: &Option<String>) -> bool {
    value
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
}

fn hf_patch_affects(patch: &HeadlessRunFormat, tag: &str) -> bool {
    match tag {
        "w:rFonts" => {
            nonempty(&patch.latin_font)
                || nonempty(&patch.east_asia_font)
                || nonempty(&patch.complex_font)
        }
        "w:sz" => patch
            .font_size
            .is_some_and(|size| size.is_finite() && size > 0.0),
        "w:szCs" => patch
            .complex_font_size
            .is_some_and(|size| size.is_finite() && size > 0.0),
        "w:b" => patch.bold.is_some(),
        "w:bCs" => patch.complex_bold.is_some(),
        "w:i" => patch.italic.is_some(),
        "w:iCs" => patch.complex_italic.is_some(),
        "w:u" => patch.underline.is_some() || patch.underline_color.is_some(),
        "w:strike" | "w:dstrike" => patch.strike.is_some() || patch.double_strike.is_some(),
        "w:vertAlign" => patch.vertical_align.is_some(),
        "w:color" => patch.color.is_some(),
        "w:caps" | "w:smallCaps" => patch.caps.is_some() || patch.small_caps.is_some(),
        "w:vanish" => patch.hidden.is_some(),
        "w:em" => patch.emphasis.is_some(),
        "w:shd" => patch.shading.is_some(),
        "w:bdr" => patch.border.is_some(),
        "w:w" => patch.scale.is_some_and(f32::is_finite),
        "w:spacing" => patch.spacing.is_some_and(f32::is_finite),
        "w:position" => patch.position.is_some_and(f32::is_finite),
        "w:kern" => patch.kerning.is_some_and(f32::is_finite),
        "w:outline" => patch.outline.is_some(),
        "w:shadow" => patch.shadow.is_some(),
        "w:emboss" | "w:imprint" => patch.emboss.is_some() || patch.engrave.is_some(),
        "w14:ligatures" => patch.ligatures.is_some(),
        "w14:numSpacing" => patch.number_spacing.is_some(),
        "w14:numForm" => patch.number_form.is_some(),
        "w14:stylisticSets" => patch.stylistic_set.is_some(),
        "w14:cntxtAlts" => patch.contextual_alternates.is_some(),
        "w14:glow" => patch.glow_color.is_some() || patch.glow_radius.is_some(),
        "w14:textOutline" => {
            patch.text_outline_color.is_some() || patch.text_outline_width.is_some()
        }
        "w:eastAsianLayout" => patch.east_asian_layout.is_some(),
        "w:lang" => patch.language.is_some() || patch.east_asia_language.is_some(),
        "w:noProof" => patch.no_proof.is_some(),
        _ => false,
    }
}
