use crate::document::{EastAsianLineBreakingSettings, EndnotePosition, Note, NoteNumbering};
use quick_xml::escape::unescape;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn local_name(name: &[u8]) -> &[u8] {
    match name.iter().position(|b| *b == b':') {
        Some(idx) if idx + 1 < name.len() => &name[idx + 1..],
        _ => name,
    }
}

fn is_no_line_break_before_tag(name: &[u8]) -> bool {
    matches!(name, b"noLineBreakBefore" | b"noLineBreaksBefore")
}

fn is_no_line_break_after_tag(name: &[u8]) -> bool {
    matches!(name, b"noLineBreakAfter" | b"noLineBreaksAfter")
}

/// Whether a `w:noLineBreaksBefore`/`w:noLineBreaksAfter` element is the
/// Japanese custom set. ECMA-376 §17.15.1.58-59: a custom set replaces the
/// application's default set for its language, and jade's default tables are
/// the Japanese ones (`jade_text_layout::kinsoku`).
fn is_japanese_custom_line_break_set(element: &quick_xml::events::BytesStart<'_>) -> bool {
    let name = element.name();
    if !matches!(
        local_name(name.as_ref()),
        b"noLineBreaksBefore" | b"noLineBreaksAfter"
    ) {
        return false;
    }
    element.attributes().flatten().any(|attr| {
        local_name(attr.key.as_ref()) == b"lang"
            && attr
                .value
                .get(..2)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"ja"))
    })
}

fn push_line_break_entries(target: &mut Vec<String>, raw: &str) {
    for ch in raw.chars() {
        if ch.is_whitespace() {
            continue;
        }
        let entry = ch.to_string();
        if !target.iter().any(|existing| existing == &entry) {
            target.push(entry);
        }
    }
}

/// The document's math properties (`m:mathPr`, ECMA-376 §22.1.2.62):
/// defaults every equation in the document lays out with.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct MathProperties {
    /// `m:mathFont`: the default math face.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub math_font: Option<String>,
    /// `m:brkBin`: before, after or repeat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub break_binary: Option<String>,
    /// `m:brkBinSub`: --, -+ or +-.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub break_binary_subtraction: Option<String>,
    /// `m:smallFrac`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub small_fractions: Option<bool>,
    /// `m:dispDef`: display math by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_default: Option<bool>,
    /// `m:lMargin` / `m:rMargin`, twips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_margin: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_margin: Option<f32>,
    /// `m:defJc`: left, right, center or centerGroup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_justification: Option<String>,
    /// `m:preSp`, `m:postSp`, `m:interSp`, `m:intraSp`, twips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_before: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space_after: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inter_equation_space: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intra_equation_space: Option<f32>,
    /// `m:wrapIndent`, twips, or `m:wrapRight`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_indent: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_right: Option<bool>,
    /// `m:intLim` / `m:naryLim`: subSup or undOvr.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integral_limits: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nary_limits: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocumentSettings {
    /// `m:mathPr`.
    #[serde(default)]
    pub math: MathProperties,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footnote_properties: Option<NoteNumbering>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endnote_properties: Option<NoteNumbering>,
    /// Special note IDs selected by document-wide `w:footnotePr/w:footnote`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub special_footnote_ids: Vec<String>,
    /// Special note IDs selected by document-wide `w:endnotePr/w:endnote`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub special_endnote_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endnote_position: Option<EndnotePosition>,
    pub snap_to_grid_default: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_tab_stop: Option<f32>,
    /// Content-revision visibility from an authored `w:revisionView`.
    /// Absence leaves the host's initial display policy unspecified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_content_revisions: Option<bool>,
    /// Mirrors the `w:noTabHangInd` compatibility switch. When false (the
    /// OOXML default), a hanging indent creates an implicit custom tab stop at
    /// the paragraph's left indent.
    #[serde(default)]
    pub no_tab_hanging_indent: bool,
    /// Mirrors the `w:adjustLineHeightInTable` compatibility switch. When
    /// false (the OOXML default), the section's line grid pitch is not applied
    /// to lines in table cells (ECMA-376 §17.15.3.1).
    #[serde(default)]
    pub adjust_line_height_in_table: bool,
    #[serde(default)]
    pub use_east_asian_layout: bool,
    #[serde(default)]
    pub balance_single_byte_double_byte_width: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kinsoku: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub word_wrap: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overflow_punctuation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_line_punctuation: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_space_de: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_space_dn: Option<bool>,
    #[serde(default)]
    pub character_spacing_control: Option<String>,
    /// `w:compatSetting[@w:name="compatibilityMode"]/@w:val`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility_mode: Option<u32>,
    #[serde(default)]
    pub no_line_break_before: Vec<String>,
    #[serde(default)]
    pub no_line_break_after: Vec<String>,
    /// A Japanese `w:noLineBreaksBefore` set replaces the default table
    /// instead of adding to it.
    #[serde(default)]
    pub no_line_break_before_replaces_default: bool,
    /// A Japanese `w:noLineBreaksAfter` set replaces the default table.
    #[serde(default)]
    pub no_line_break_after_replaces_default: bool,
    /// `<w:strictFirstAndLastChars/>`.
    #[serde(default)]
    pub strict_first_and_last_chars: bool,
    /// Mirrors `<w:evenAndOddHeaders/>`. When true, sections that declare an
    /// even-page header/footer reference use it on even-numbered pages.
    #[serde(default)]
    pub even_and_odd_headers: bool,
    /// Adds the section gutter to the top margin instead of the left edge.
    #[serde(default)]
    pub gutter_at_top: bool,
    /// Swaps physical left/right margins on even pages.
    #[serde(default)]
    pub mirror_margins: bool,
    /// `w:bookFoldPrinting`.
    #[serde(default)]
    pub book_fold_printing: bool,
    /// `w:printTwoOnOne`.
    #[serde(default)]
    pub print_two_on_one: bool,
    /// Automatic hyphenation (`w:autoHyphenation` and its companions).
    #[serde(default)]
    pub hyphenation: crate::document::HyphenationSettings,
    /// Values declared by `w:docVars/w:docVar`. Keys are normalized for the
    /// case-insensitive lookup Word applies to `DOCVARIABLE` instructions.
    #[serde(default)]
    pub document_variables: HashMap<String, String>,
}

impl DocumentSettings {
    /// ISO/IEC 29500 §17.11.9: unlisted special notes must not be loaded.
    /// Ordinary notes remain active independently of the special-note list.
    pub fn is_note_active(&self, note: &Note, is_footnote: bool) -> bool {
        if note
            .note_type
            .as_deref()
            .is_none_or(|kind| kind.eq_ignore_ascii_case("normal"))
        {
            return true;
        }
        let ids = if is_footnote {
            &self.special_footnote_ids
        } else {
            &self.special_endnote_ids
        };
        ids.contains(&note.id)
    }

    fn parse_note_properties_child(
        numbering: &mut NoteNumbering,
        special_ids: &mut Vec<String>,
        note_tag: &[u8],
        name: &[u8],
        element: &quick_xml::events::BytesStart<'_>,
    ) {
        if name == note_tag {
            if let Some(id) = element
                .attributes()
                .flatten()
                .find(|attr| local_name(attr.key.as_ref()) == b"id")
            {
                let id = String::from_utf8_lossy(&id.value).trim().to_string();
                if !id.is_empty() && !special_ids.contains(&id) {
                    special_ids.push(id);
                }
            }
            return;
        }
        let Some(value) = element
            .attributes()
            .flatten()
            .find(|attr| local_name(attr.key.as_ref()) == b"val")
        else {
            return;
        };
        let value = String::from_utf8_lossy(&value.value).trim().to_string();
        match name {
            b"numFmt" if !value.is_empty() => numbering.number_format = Some(value.to_lowercase()),
            b"numStart" => numbering.number_start = value.parse().ok(),
            b"numRestart" if !value.is_empty() => {
                numbering.number_restart = Some(value.to_lowercase())
            }
            _ => {}
        }
    }

    fn parse_endnote_position(
        element: &quick_xml::events::BytesStart<'_>,
    ) -> Option<EndnotePosition> {
        let value = element
            .attributes()
            .flatten()
            .find(|attr| local_name(attr.key.as_ref()) == b"val")?;
        match value.value.as_ref() {
            b"sectEnd" => Some(EndnotePosition::SectionEnd),
            b"docEnd" => Some(EndnotePosition::DocumentEnd),
            _ => None,
        }
    }

    /// The `w:val` of a `compatSetting` named `compatibilityMode`.
    fn parse_compatibility_mode(element: &quick_xml::events::BytesStart<'_>) -> Option<u32> {
        let mut is_mode = false;
        let mut value = None;
        for attr in element.attributes().flatten() {
            match local_name(attr.key.as_ref()) {
                b"name" => is_mode = attr.value.as_ref() == b"compatibilityMode",
                b"val" => value = String::from_utf8_lossy(&attr.value).trim().parse().ok(),
                _ => {}
            }
        }
        if is_mode {
            value
        } else {
            None
        }
    }

    fn parse_revision_content_visibility(
        element: &quick_xml::events::BytesStart<'_>,
    ) -> Option<bool> {
        let Some(attr) = element
            .attributes()
            .flatten()
            .find(|attr| local_name(attr.key.as_ref()) == b"insDel")
        else {
            return Some(true);
        };
        match attr.value.as_ref() {
            b"1" | b"true" | b"on" => Some(true),
            b"0" | b"false" | b"off" => Some(false),
            _ => None,
        }
    }

    fn parse_on_off_value(value: &str) -> bool {
        !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "off"
        )
    }

    fn parse_on_off_attribute<'a, I>(attrs: I) -> bool
    where
        I: IntoIterator<Item = quick_xml::events::attributes::Attribute<'a>>,
    {
        for attr in attrs {
            if local_name(attr.key.as_ref()) == b"val" {
                return Self::parse_on_off_value(&String::from_utf8_lossy(&attr.value));
            }
        }
        true
    }

    /// One child of `m:mathPr`.
    fn parse_math_property<'a>(
        math: &mut MathProperties,
        name: &[u8],
        attributes: impl Iterator<Item = quick_xml::events::attributes::Attribute<'a>>,
    ) {
        let value = attributes
            .filter(|attr| local_name(attr.key.as_ref()) == b"val")
            .map(|attr| String::from_utf8_lossy(&attr.value).to_string())
            .next();
        let on = || {
            value
                .as_deref()
                .is_none_or(|v| matches!(v, "1" | "true" | "on"))
        };
        let twips = || value.as_deref().and_then(|v| v.parse::<f32>().ok());
        match name {
            b"mathFont" => math.math_font = value.clone(),
            b"brkBin" => math.break_binary = value.clone(),
            b"brkBinSub" => math.break_binary_subtraction = value.clone(),
            b"smallFrac" => math.small_fractions = Some(on()),
            b"dispDef" => math.display_default = Some(on()),
            b"lMargin" => math.left_margin = twips(),
            b"rMargin" => math.right_margin = twips(),
            b"defJc" => math.default_justification = value.clone(),
            b"preSp" => math.space_before = twips(),
            b"postSp" => math.space_after = twips(),
            b"interSp" => math.inter_equation_space = twips(),
            b"intraSp" => math.intra_equation_space = twips(),
            b"wrapIndent" => math.wrap_indent = twips(),
            b"wrapRight" => math.wrap_right = Some(on()),
            b"intLim" => math.integral_limits = value.clone(),
            b"naryLim" => math.nary_limits = value.clone(),
            _ => {}
        }
    }

    /// Reads one of the hyphenation settings (ECMA-376 §17.15.1.10, .21,
    /// .53, .37) into `hyphenation`; false when `name` is none of them.
    fn parse_hyphenation_setting<'a, I>(
        name: &[u8],
        attrs: I,
        hyphenation: &mut crate::document::HyphenationSettings,
    ) -> bool
    where
        I: IntoIterator<Item = quick_xml::events::attributes::Attribute<'a>>,
    {
        let number = |attrs: I| {
            attrs
                .into_iter()
                .find(|attr| local_name(attr.key.as_ref()) == b"val")
                .and_then(|attr| {
                    String::from_utf8_lossy(&attr.value)
                        .trim()
                        .parse::<u32>()
                        .ok()
                })
        };
        match name {
            b"autoHyphenation" => hyphenation.auto = Self::parse_on_off_attribute(attrs),
            b"doNotHyphenateCaps" => {
                hyphenation.do_not_hyphenate_caps = Self::parse_on_off_attribute(attrs)
            }
            b"hyphenationZone" => hyphenation.zone_twips = number(attrs),
            b"consecutiveHyphenLimit" => hyphenation.consecutive_limit = number(attrs),
            _ => return false,
        }
        true
    }

    fn parse_twips_attribute<'a, I>(attrs: I) -> Option<f32>
    where
        I: IntoIterator<Item = quick_xml::events::attributes::Attribute<'a>>,
    {
        attrs
            .into_iter()
            .find(|attr| local_name(attr.key.as_ref()) == b"val")
            .and_then(|attr| String::from_utf8_lossy(&attr.value).parse::<f32>().ok())
            .map(|value| value / 20.0)
    }

    fn parse_document_variable<'a, I>(attrs: I) -> Option<(String, String)>
    where
        I: IntoIterator<Item = quick_xml::events::attributes::Attribute<'a>>,
    {
        let mut name = None;
        let mut value = None;
        for attr in attrs {
            let decoded = attr.unescape_value().ok()?.into_owned();
            match local_name(attr.key.as_ref()) {
                b"name" => name = Some(decoded),
                b"val" => value = Some(decoded),
                _ => {}
            }
        }
        let name = name?.trim().to_string();
        if name.is_empty() {
            return None;
        }
        Some((name.to_ascii_lowercase(), value.unwrap_or_default()))
    }

    pub fn from_xml(xml: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        let mut settings = DocumentSettings::default();
        let mut depth = 0usize;
        let mut endnote_properties_depth = None;
        let mut footnote_properties_depth = None;
        let mut compat_depth = 0usize;
        let mut inside_no_line_break_before = false;
        let mut inside_no_line_break_after = false;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    depth += 1;
                    let raw = e.name().as_ref().to_vec();
                    let name = local_name(&raw);
                    if name == b"endnotePr" && depth == 2 {
                        endnote_properties_depth = Some(depth);
                        settings.endnote_properties = Some(NoteNumbering::default());
                    } else if name == b"footnotePr" && depth == 2 {
                        footnote_properties_depth = Some(depth);
                        settings.footnote_properties = Some(NoteNumbering::default());
                    } else if name == b"pos" && endnote_properties_depth == Some(depth - 1) {
                        settings.endnote_position = Self::parse_endnote_position(e);
                    }
                    if endnote_properties_depth == Some(depth - 1) {
                        Self::parse_note_properties_child(
                            settings.endnote_properties.as_mut().unwrap(),
                            &mut settings.special_endnote_ids,
                            b"endnote",
                            name,
                            e,
                        );
                    } else if footnote_properties_depth == Some(depth - 1) {
                        Self::parse_note_properties_child(
                            settings.footnote_properties.as_mut().unwrap(),
                            &mut settings.special_footnote_ids,
                            b"footnote",
                            name,
                            e,
                        );
                    }
                    if depth == 2 && name == b"revisionView" {
                        settings.display_content_revisions =
                            Self::parse_revision_content_visibility(e);
                    }
                    Self::parse_math_property(&mut settings.math, name, e.attributes().flatten());
                    if name == b"compat" {
                        compat_depth += 1;
                    } else if name == b"kinsoku" {
                        settings.kinsoku =
                            Some(Self::parse_on_off_attribute(e.attributes().flatten()));
                    } else if name == b"wordWrap" {
                        settings.word_wrap =
                            Some(Self::parse_on_off_attribute(e.attributes().flatten()));
                    } else if name == b"overflowPunct" {
                        settings.overflow_punctuation =
                            Some(Self::parse_on_off_attribute(e.attributes().flatten()));
                    } else if name == b"topLinePunct" {
                        settings.top_line_punctuation =
                            Some(Self::parse_on_off_attribute(e.attributes().flatten()));
                    } else if name == b"autoSpaceDE" {
                        settings.auto_space_de =
                            Some(Self::parse_on_off_attribute(e.attributes().flatten()));
                    } else if name == b"autoSpaceDN" {
                        settings.auto_space_dn =
                            Some(Self::parse_on_off_attribute(e.attributes().flatten()));
                    } else if name == b"characterSpacingControl" {
                        if let Some(value) = e
                            .attributes()
                            .flatten()
                            .find(|attr| local_name(attr.key.as_ref()) == b"val")
                        {
                            settings.character_spacing_control =
                                Some(String::from_utf8_lossy(&value.value).to_string());
                        }
                    } else if name == b"strictFirstAndLastChars" {
                        settings.strict_first_and_last_chars =
                            Self::parse_on_off_attribute(e.attributes().flatten());
                    } else if name == b"evenAndOddHeaders" {
                        settings.even_and_odd_headers =
                            Self::parse_on_off_attribute(e.attributes().flatten());
                    } else if name == b"gutterAtTop" {
                        settings.gutter_at_top =
                            Self::parse_on_off_attribute(e.attributes().flatten());
                    } else if name == b"mirrorMargins" {
                        settings.mirror_margins =
                            Self::parse_on_off_attribute(e.attributes().flatten());
                    } else if name == b"bookFoldPrinting" {
                        settings.book_fold_printing =
                            Self::parse_on_off_attribute(e.attributes().flatten());
                    } else if name == b"printTwoOnOne" {
                        settings.print_two_on_one =
                            Self::parse_on_off_attribute(e.attributes().flatten());
                    } else if Self::parse_hyphenation_setting(
                        name,
                        e.attributes().flatten(),
                        &mut settings.hyphenation,
                    ) {
                    } else if name == b"defaultTabStop" {
                        settings.default_tab_stop =
                            Self::parse_twips_attribute(e.attributes().flatten());
                    } else if name == b"docVar" {
                        if let Some((name, value)) =
                            Self::parse_document_variable(e.attributes().flatten())
                        {
                            settings.document_variables.insert(name, value);
                        }
                    }
                    if is_no_line_break_before_tag(name) {
                        if is_japanese_custom_line_break_set(e) {
                            settings.no_line_break_before_replaces_default = true;
                        }
                        inside_no_line_break_before = true;
                        if let Some(value) = e
                            .attributes()
                            .flatten()
                            .find(|attr| local_name(attr.key.as_ref()) == b"val")
                        {
                            push_line_break_entries(
                                &mut settings.no_line_break_before,
                                &String::from_utf8_lossy(&value.value),
                            );
                        }
                    } else if is_no_line_break_after_tag(name) {
                        inside_no_line_break_after = true;
                        if is_japanese_custom_line_break_set(e) {
                            settings.no_line_break_after_replaces_default = true;
                        }
                        if let Some(value) = e
                            .attributes()
                            .flatten()
                            .find(|attr| local_name(attr.key.as_ref()) == b"val")
                        {
                            push_line_break_entries(
                                &mut settings.no_line_break_after,
                                &String::from_utf8_lossy(&value.value),
                            );
                        }
                    }
                    if compat_depth > 0 {
                        match name {
                            b"useFELayout" => {
                                let enabled =
                                    Self::parse_on_off_attribute(e.attributes().flatten());
                                settings.snap_to_grid_default = enabled;
                                settings.use_east_asian_layout = enabled;
                            }
                            b"balanceSingleByteDoubleByteWidth" => {
                                settings.balance_single_byte_double_byte_width =
                                    Self::parse_on_off_attribute(e.attributes().flatten());
                            }
                            b"noTabHangInd" => {
                                settings.no_tab_hanging_indent =
                                    Self::parse_on_off_attribute(e.attributes().flatten());
                            }
                            b"adjustLineHeightInTable" => {
                                settings.adjust_line_height_in_table =
                                    Self::parse_on_off_attribute(e.attributes().flatten());
                            }
                            b"characterSpacingControl" => {
                                if let Some(value) = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| local_name(attr.key.as_ref()) == b"val")
                                {
                                    settings.character_spacing_control =
                                        Some(String::from_utf8_lossy(&value.value).to_string());
                                }
                            }
                            b"compatSetting" => {
                                if let Some(mode) = Self::parse_compatibility_mode(e) {
                                    settings.compatibility_mode = Some(mode);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Event::Empty(ref e)) => {
                    let raw = e.name().as_ref().to_vec();
                    let name = local_name(&raw);
                    if depth == 1 && name == b"revisionView" {
                        settings.display_content_revisions =
                            Self::parse_revision_content_visibility(e);
                    }
                    if depth == 1 && name == b"endnotePr" {
                        settings.endnote_properties = Some(NoteNumbering::default());
                    } else if depth == 1 && name == b"footnotePr" {
                        settings.footnote_properties = Some(NoteNumbering::default());
                    }
                    if endnote_properties_depth == Some(depth) {
                        Self::parse_note_properties_child(
                            settings.endnote_properties.as_mut().unwrap(),
                            &mut settings.special_endnote_ids,
                            b"endnote",
                            name,
                            e,
                        );
                    } else if footnote_properties_depth == Some(depth) {
                        Self::parse_note_properties_child(
                            settings.footnote_properties.as_mut().unwrap(),
                            &mut settings.special_footnote_ids,
                            b"footnote",
                            name,
                            e,
                        );
                    }
                    if name == b"pos" && endnote_properties_depth == Some(depth) {
                        settings.endnote_position = Self::parse_endnote_position(e);
                    }
                    if name == b"characterSpacingControl" {
                        if let Some(value) = e
                            .attributes()
                            .flatten()
                            .find(|attr| local_name(attr.key.as_ref()) == b"val")
                        {
                            settings.character_spacing_control =
                                Some(String::from_utf8_lossy(&value.value).to_string());
                        }
                    }
                    if name == b"kinsoku" {
                        settings.kinsoku =
                            Some(Self::parse_on_off_attribute(e.attributes().flatten()));
                    } else if name == b"wordWrap" {
                        settings.word_wrap =
                            Some(Self::parse_on_off_attribute(e.attributes().flatten()));
                    } else if name == b"overflowPunct" {
                        settings.overflow_punctuation =
                            Some(Self::parse_on_off_attribute(e.attributes().flatten()));
                    } else if name == b"topLinePunct" {
                        settings.top_line_punctuation =
                            Some(Self::parse_on_off_attribute(e.attributes().flatten()));
                    }
                    Self::parse_math_property(&mut settings.math, name, e.attributes().flatten());
                    if name == b"autoSpaceDE" {
                        settings.auto_space_de =
                            Some(Self::parse_on_off_attribute(e.attributes().flatten()));
                    } else if name == b"autoSpaceDN" {
                        settings.auto_space_dn =
                            Some(Self::parse_on_off_attribute(e.attributes().flatten()));
                    } else if name == b"strictFirstAndLastChars" {
                        settings.strict_first_and_last_chars =
                            Self::parse_on_off_attribute(e.attributes().flatten());
                    } else if name == b"evenAndOddHeaders" {
                        settings.even_and_odd_headers =
                            Self::parse_on_off_attribute(e.attributes().flatten());
                    } else if name == b"gutterAtTop" {
                        settings.gutter_at_top =
                            Self::parse_on_off_attribute(e.attributes().flatten());
                    } else if name == b"mirrorMargins" {
                        settings.mirror_margins =
                            Self::parse_on_off_attribute(e.attributes().flatten());
                    } else if name == b"bookFoldPrinting" {
                        settings.book_fold_printing =
                            Self::parse_on_off_attribute(e.attributes().flatten());
                    } else if name == b"printTwoOnOne" {
                        settings.print_two_on_one =
                            Self::parse_on_off_attribute(e.attributes().flatten());
                    } else if Self::parse_hyphenation_setting(
                        name,
                        e.attributes().flatten(),
                        &mut settings.hyphenation,
                    ) {
                    } else if name == b"defaultTabStop" {
                        settings.default_tab_stop =
                            Self::parse_twips_attribute(e.attributes().flatten());
                    } else if name == b"docVar" {
                        if let Some((name, value)) =
                            Self::parse_document_variable(e.attributes().flatten())
                        {
                            settings.document_variables.insert(name, value);
                        }
                    }
                    if is_no_line_break_before_tag(name) {
                        if is_japanese_custom_line_break_set(e) {
                            settings.no_line_break_before_replaces_default = true;
                        }
                        if let Some(value) = e
                            .attributes()
                            .flatten()
                            .find(|attr| local_name(attr.key.as_ref()) == b"val")
                        {
                            push_line_break_entries(
                                &mut settings.no_line_break_before,
                                &String::from_utf8_lossy(&value.value),
                            );
                        }
                    } else if is_no_line_break_after_tag(name) {
                        if is_japanese_custom_line_break_set(e) {
                            settings.no_line_break_after_replaces_default = true;
                        }
                        if let Some(value) = e
                            .attributes()
                            .flatten()
                            .find(|attr| local_name(attr.key.as_ref()) == b"val")
                        {
                            push_line_break_entries(
                                &mut settings.no_line_break_after,
                                &String::from_utf8_lossy(&value.value),
                            );
                        }
                    }
                    if compat_depth > 0 {
                        match name {
                            b"useFELayout" => {
                                let enabled =
                                    Self::parse_on_off_attribute(e.attributes().flatten());
                                settings.snap_to_grid_default = enabled;
                                settings.use_east_asian_layout = enabled;
                            }
                            b"balanceSingleByteDoubleByteWidth" => {
                                settings.balance_single_byte_double_byte_width =
                                    Self::parse_on_off_attribute(e.attributes().flatten());
                            }
                            b"noTabHangInd" => {
                                settings.no_tab_hanging_indent =
                                    Self::parse_on_off_attribute(e.attributes().flatten());
                            }
                            b"adjustLineHeightInTable" => {
                                settings.adjust_line_height_in_table =
                                    Self::parse_on_off_attribute(e.attributes().flatten());
                            }
                            b"characterSpacingControl" => {
                                if let Some(value) = e
                                    .attributes()
                                    .flatten()
                                    .find(|attr| local_name(attr.key.as_ref()) == b"val")
                                {
                                    settings.character_spacing_control =
                                        Some(String::from_utf8_lossy(&value.value).to_string());
                                }
                            }
                            b"compatSetting" => {
                                if let Some(mode) = Self::parse_compatibility_mode(e) {
                                    settings.compatibility_mode = Some(mode);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Event::Text(ref e)) => {
                    if compat_depth > 0 {
                        let decoded = e
                            .decode()
                            .map_err(|err| format!("settings.xml parse error: {err}"))?;
                        let text = unescape(decoded.as_ref())
                            .map_err(|err| format!("settings.xml parse error: {err}"))?
                            .trim()
                            .to_string();
                        if text.is_empty() {
                            continue;
                        }
                        if inside_no_line_break_before {
                            push_line_break_entries(&mut settings.no_line_break_before, &text);
                        } else if inside_no_line_break_after {
                            push_line_break_entries(&mut settings.no_line_break_after, &text);
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let raw = e.name().as_ref().to_vec();
                    let name = local_name(&raw);
                    if endnote_properties_depth == Some(depth) {
                        endnote_properties_depth = None;
                    }
                    if footnote_properties_depth == Some(depth) {
                        footnote_properties_depth = None;
                    }
                    depth = depth.saturating_sub(1);
                    if name == b"compat" && compat_depth > 0 {
                        compat_depth -= 1;
                    } else if is_no_line_break_before_tag(name) {
                        inside_no_line_break_before = false;
                    } else if is_no_line_break_after_tag(name) {
                        inside_no_line_break_after = false;
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("settings.xml parse error: {err}").into()),
                _ => {}
            }
        }

        Ok(settings)
    }

    pub fn to_line_breaking_settings(&self) -> EastAsianLineBreakingSettings {
        EastAsianLineBreakingSettings {
            use_east_asian_layout: self.use_east_asian_layout,
            balance_single_byte_double_byte_width: self.balance_single_byte_double_byte_width,
            kinsoku: self.kinsoku,
            word_wrap: self.word_wrap,
            overflow_punctuation: self.overflow_punctuation,
            top_line_punctuation: self.top_line_punctuation,
            auto_space_de: self.auto_space_de,
            auto_space_dn: self.auto_space_dn,
            character_spacing_control: self.character_spacing_control.clone(),
            no_line_break_before: self.no_line_break_before.clone(),
            no_line_break_after: self.no_line_break_after.clone(),
            no_line_break_before_replaces_default: self.no_line_break_before_replaces_default,
            no_line_break_after_replaces_default: self.no_line_break_after_replaces_default,
            strict_first_and_last_chars: self.strict_first_and_last_chars,
            compatibility_mode: self.compatibility_mode,
        }
    }
}

#[cfg(test)]
mod saved_revision_view_tests {
    use super::DocumentSettings;
    #[test]
    fn saved_revision_view_distinguishes_absence_default_and_content_visibility() {
        for (child, expected) in [
            ("", None),
            ("<w:revisionView/>", Some(true)),
            (
                "<w:revisionView w:markup=\"false\" w:insDel=\"true\"/>",
                Some(true),
            ),
            (
                "<w:revisionView w:markup=\"true\" w:insDel=\"false\"/>",
                Some(false),
            ),
            (
                "<w:revisionView w:insDel=\"off\"></w:revisionView>",
                Some(false),
            ),
            ("<w:revisionView w:insDel=\"bogus\"/>", None),
            (
                "<w:compat><w:revisionView w:insDel=\"true\"/></w:compat>",
                None,
            ),
        ] {
            let xml = format!("<w:settings xmlns:w=\"urn:w\">{child}</w:settings>");
            assert_eq!(
                DocumentSettings::from_xml(&xml)
                    .unwrap()
                    .display_content_revisions,
                expected,
                "{child}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_properties_are_read_from_math_pr() {
        let xml = r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math"><m:mathPr><m:mathFont m:val="Cambria Math"/><m:brkBin m:val="after"/><m:brkBinSub m:val="--"/><m:smallFrac m:val="0"/><m:dispDef/><m:lMargin m:val="0"/><m:rMargin m:val="0"/><m:defJc m:val="centerGroup"/><m:wrapIndent m:val="1440"/><m:intLim m:val="undOvr"/><m:naryLim m:val="subSup"/></m:mathPr></w:settings>"#;
        let math = DocumentSettings::from_xml(xml).unwrap().math;
        assert_eq!(math.math_font.as_deref(), Some("Cambria Math"));
        assert_eq!(math.break_binary.as_deref(), Some("after"));
        assert_eq!(math.small_fractions, Some(false));
        assert_eq!(math.display_default, Some(true));
        assert_eq!(math.wrap_indent, Some(1440.0));
        assert_eq!(math.integral_limits.as_deref(), Some("undOvr"));
        assert_eq!(math.nary_limits.as_deref(), Some("subSup"));
        assert_eq!(math.default_justification.as_deref(), Some("centerGroup"));
    }

    #[test]
    fn special_note_lists_select_only_direct_matching_references() {
        for (properties, note_tag, is_footnote) in [
            ("footnotePr", "footnote", true),
            ("endnotePr", "endnote", false),
        ] {
            for reference in [
                format!(r#"<n:{note_tag} n:id="-1"/>"#),
                format!(r#"<n:{note_tag} n:id="-1"></n:{note_tag}>"#),
            ] {
                let wrong_tag = if is_footnote { "endnote" } else { "footnote" };
                let xml = format!(
                    r#"<n:settings xmlns:n="urn:test"><n:{properties}>{reference}<n:{note_tag} n:id="0"/><n:{note_tag} n:id="0"/><n:{note_tag}/><n:{wrong_tag} n:id="2"/><n:extension><n:{note_tag} n:id="3"/></n:extension></n:{properties}><n:{note_tag} n:id="4"/></n:settings>"#
                );
                let settings = DocumentSettings::from_xml(&xml).unwrap();
                let ids = if is_footnote {
                    &settings.special_footnote_ids
                } else {
                    &settings.special_endnote_ids
                };
                assert_eq!(ids, &["-1", "0"]);
                for id in ["-1", "0", "1", "2", "3", "4"] {
                    let note = Note {
                        id: id.into(),
                        note_type: Some("continuationNotice".into()),
                        ..Note::default()
                    };
                    assert_eq!(
                        settings.is_note_active(&note, is_footnote),
                        matches!(id, "-1" | "0")
                    );
                    assert!(!settings.is_note_active(&note, !is_footnote));
                }
                for kind in [None, Some("normal".into())] {
                    assert!(settings.is_note_active(
                        &Note {
                            id: "9".into(),
                            note_type: kind,
                            ..Note::default()
                        },
                        is_footnote
                    ));
                }
            }
        }
        assert!(!DocumentSettings::default().is_note_active(
            &Note {
                id: "1".into(),
                note_type: Some("continuationNotice".into()),
                ..Note::default()
            },
            true
        ));
    }

    #[test]
    fn parses_document_wide_note_numbering_without_nested_overrides() {
        for tag in ["footnotePr", "endnotePr"] {
            for close in ["/>", "></n:numFmt>"] {
                let xml = format!(
                    r#"<n:settings xmlns:n="urn:test"><n:{tag}><n:numFmt n:val="upperRoman"{close}<n:numStart n:val="8"/><n:numRestart n:val="eachSect"/><n:extension><n:numStart n:val="99"/></n:extension></n:{tag}><n:numStart n:val="77"/></n:settings>"#
                );
                let settings = DocumentSettings::from_xml(&xml).unwrap();
                let (numbering, other) = if tag == "footnotePr" {
                    (settings.footnote_properties, settings.endnote_properties)
                } else {
                    (settings.endnote_properties, settings.footnote_properties)
                };
                assert_eq!(
                    numbering,
                    Some(crate::document::NoteNumbering {
                        number_format: Some("upperroman".into()),
                        number_start: Some(8),
                        number_restart: Some("eachsect".into()),
                    })
                );
                assert_eq!(other, None);
            }
        }
        let settings = DocumentSettings::from_xml(r#"<w:settings xmlns:w="urn:test"><w:footnotePr><w:numStart w:val="invalid"/></w:footnotePr><w:endnotePr/></w:settings>"#).unwrap();
        assert!(settings.footnote_properties.unwrap().is_empty());
        assert!(settings.endnote_properties.unwrap().is_empty());
    }

    #[test]
    fn parses_document_wide_endnote_position() {
        for (value, expected) in [
            (
                "sectEnd",
                Some(crate::document::EndnotePosition::SectionEnd),
            ),
            (
                "docEnd",
                Some(crate::document::EndnotePosition::DocumentEnd),
            ),
            ("invalid", None),
        ] {
            for position in [
                format!(r#"<w:pos w:val="{value}"/>"#),
                format!(r#"<w:pos w:val="{value}"></w:pos>"#),
            ] {
                let xml = format!(
                    r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:footnotePr><w:pos w:val="sectEnd"/></w:footnotePr><w:endnotePr>{position}</w:endnotePr></w:settings>"#
                );
                assert_eq!(
                    DocumentSettings::from_xml(&xml).unwrap().endnote_position,
                    expected
                );
            }
        }
        assert_eq!(
            DocumentSettings::from_xml(r#"<w:settings xmlns:w="urn:test"><w:footnotePr><w:pos w:val="sectEnd"/></w:footnotePr></w:settings>"#)
                .unwrap().endnote_position,
            None,
        );
        assert_eq!(
            DocumentSettings::from_xml(r#"<w:settings xmlns:w="urn:test"><w:endnotePr><w:pos w:val="docEnd"/><w:extension><w:pos w:val="sectEnd"/></w:extension></w:endnotePr><w:pos w:val="sectEnd"/></w:settings>"#)
                .unwrap().endnote_position,
            Some(crate::document::EndnotePosition::DocumentEnd),
        );
    }

    #[test]
    fn parses_east_asian_line_break_settings() {
        let xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:compat>
            <w:useFELayout/>
            <w:balanceSingleByteDoubleByteWidth/>
            <w:characterSpacingControl w:val="doNotCompress"/>
            <w:noLineBreakBefore w:val="、"/>
            <w:noLineBreakAfter>。</w:noLineBreakAfter>
          </w:compat>
        </w:settings>
        "#;

        let settings = DocumentSettings::from_xml(xml).expect("parse settings");
        assert!(settings.snap_to_grid_default);
        assert!(settings.use_east_asian_layout);
        assert!(settings.balance_single_byte_double_byte_width);
        assert_eq!(
            settings.character_spacing_control.as_deref(),
            Some("doNotCompress")
        );
        assert_eq!(settings.no_line_break_before, vec!["、".to_string()]);
        assert_eq!(settings.no_line_break_after, vec!["。".to_string()]);

        let converted = settings.to_line_breaking_settings();
        assert!(converted.use_east_asian_layout);
        assert!(converted.balance_single_byte_double_byte_width);
        assert_eq!(
            converted.character_spacing_control.as_deref(),
            Some("doNotCompress")
        );
        assert_eq!(converted.no_line_break_before, vec!["、"]);
        assert_eq!(converted.no_line_break_after, vec!["。"]);
    }

    #[test]
    fn parses_character_spacing_control_outside_compat() {
        let xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:characterSpacingControl w:val="compressPunctuation"/>
        </w:settings>
        "#;

        let settings = DocumentSettings::from_xml(xml).expect("parse settings");
        assert_eq!(
            settings.character_spacing_control.as_deref(),
            Some("compressPunctuation")
        );
    }

    #[test]
    fn parses_character_spacing_control_start_element() {
        let xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:characterSpacingControl w:val="doNotCompress"></w:characterSpacingControl>
        </w:settings>
        "#;

        let settings = DocumentSettings::from_xml(xml).expect("parse settings");
        assert_eq!(
            settings.character_spacing_control.as_deref(),
            Some("doNotCompress")
        );
    }

    #[test]
    fn parses_default_tab_stop_twips() {
        let xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:defaultTabStop w:val="840"/>
        </w:settings>
        "#;

        let settings = DocumentSettings::from_xml(xml).expect("parse settings");
        assert_eq!(settings.default_tab_stop, Some(42.0));
    }

    #[test]
    fn parses_no_line_breaks_plural_tags_and_splits_entries() {
        let xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:noLineBreaksBefore w:lang="ja-JP" w:val="、。」"/>
          <w:noLineBreaksAfter w:lang="ja-JP" w:val="（「"/>
        </w:settings>
        "#;

        let settings = DocumentSettings::from_xml(xml).expect("parse settings");
        assert_eq!(
            settings.no_line_break_before,
            vec!["、".to_string(), "。".to_string(), "」".to_string()]
        );
        assert_eq!(
            settings.no_line_break_after,
            vec!["（".to_string(), "「".to_string()]
        );
    }

    #[test]
    fn japanese_custom_line_break_sets_replace_the_default_tables() {
        let settings = |body: &str| {
            DocumentSettings::from_xml(&format!(
                r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{body}</w:settings>"#
            ))
            .expect("parse settings")
            .to_line_breaking_settings()
        };

        let japanese = settings(
            r#"<w:noLineBreaksBefore w:lang="ja-JP" w:val="、。"/><w:noLineBreaksAfter w:lang="ja-JP" w:val="「"/>"#,
        );
        assert!(japanese.no_line_break_before_replaces_default);
        assert!(japanese.no_line_break_after_replaces_default);

        let chinese = settings(r#"<w:noLineBreaksBefore w:lang="zh-CN" w:val="、"/>"#);
        assert!(!chinese.no_line_break_before_replaces_default);
        assert!(!chinese.no_line_break_after_replaces_default);

        let legacy = settings(
            r#"<w:compat><w:noLineBreakBefore w:val="、"/><w:noLineBreakAfter>。</w:noLineBreakAfter></w:compat>"#,
        );
        assert!(!legacy.no_line_break_before_replaces_default);
        assert!(!legacy.no_line_break_after_replaces_default);
    }

    #[test]
    fn parses_auto_space_de_dn_flags() {
        let xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:autoSpaceDE w:val="0"/>
          <w:autoSpaceDN/>
        </w:settings>
        "#;

        let settings = DocumentSettings::from_xml(xml).expect("parse settings");
        assert_eq!(settings.auto_space_de, Some(false));
        assert_eq!(settings.auto_space_dn, Some(true));

        let converted = settings.to_line_breaking_settings();
        assert_eq!(converted.auto_space_de, Some(false));
        assert_eq!(converted.auto_space_dn, Some(true));
    }

    #[test]
    fn parses_compat_flags_with_explicit_off_values() {
        let xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:compat>
            <w:useFELayout w:val="0"/>
            <w:balanceSingleByteDoubleByteWidth w:val="false"/>
          </w:compat>
        </w:settings>
        "#;

        let settings = DocumentSettings::from_xml(xml).expect("parse settings");
        assert!(!settings.snap_to_grid_default);
        assert!(!settings.use_east_asian_layout);
        assert!(!settings.balance_single_byte_double_byte_width);
    }

    #[test]
    fn parses_no_tab_hanging_indent_compatibility_switch() {
        let enabled = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:compat><w:noTabHangInd/></w:compat>
        </w:settings>
        "#;
        let disabled = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:compat><w:noTabHangInd w:val="0"/></w:compat>
        </w:settings>
        "#;

        assert!(
            DocumentSettings::from_xml(enabled)
                .expect("parse enabled setting")
                .no_tab_hanging_indent
        );
        assert!(
            !DocumentSettings::from_xml(disabled)
                .expect("parse disabled setting")
                .no_tab_hanging_indent
        );
    }

    #[test]
    fn parses_adjust_line_height_in_table_compatibility_switch() {
        let parse = |compat: &str| {
            let xml = format!(
                r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{compat}</w:settings>"#
            );
            DocumentSettings::from_xml(&xml)
                .expect("parse settings")
                .adjust_line_height_in_table
        };

        assert!(!parse(""));
        assert!(parse("<w:compat><w:adjustLineHeightInTable/></w:compat>"));
        assert!(parse(
            "<w:compat><w:adjustLineHeightInTable></w:adjustLineHeightInTable></w:compat>"
        ));
        assert!(!parse(
            r#"<w:compat><w:adjustLineHeightInTable w:val="0"/></w:compat>"#
        ));
        // Outside `w:compat` the element is not the compatibility switch.
        assert!(!parse("<w:adjustLineHeightInTable/>"));
    }

    #[test]
    fn parses_even_and_odd_headers_toggle() {
        let on_xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:evenAndOddHeaders/>
        </w:settings>
        "#;
        let settings = DocumentSettings::from_xml(on_xml).expect("parse settings");
        assert!(settings.even_and_odd_headers);

        let off_xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:evenAndOddHeaders w:val="0"/>
        </w:settings>
        "#;
        let settings = DocumentSettings::from_xml(off_xml).expect("parse settings");
        assert!(!settings.even_and_odd_headers);

        let absent = DocumentSettings::from_xml(
            r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#,
        )
        .expect("parse settings");
        assert!(!absent.even_and_odd_headers);
    }

    #[test]
    fn parses_gutter_at_top_toggle() {
        let on_xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:gutterAtTop/>
        </w:settings>
        "#;
        let settings = DocumentSettings::from_xml(on_xml).expect("parse settings");
        assert!(settings.gutter_at_top);

        let off_xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:gutterAtTop w:val="false"></w:gutterAtTop>
        </w:settings>
        "#;
        let settings = DocumentSettings::from_xml(off_xml).expect("parse settings");
        assert!(!settings.gutter_at_top);
    }

    #[test]
    fn parses_hyphenation_settings() {
        let settings = DocumentSettings::from_xml(
            r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:autoHyphenation/>
                <w:consecutiveHyphenLimit w:val="2"/>
                <w:hyphenationZone w:val="425"/>
                <w:doNotHyphenateCaps/>
            </w:settings>"#,
        )
        .unwrap();
        assert!(settings.hyphenation.auto);
        assert_eq!(settings.hyphenation.consecutive_limit, Some(2));
        assert_eq!(settings.hyphenation.zone_twips, Some(425));
        assert!(settings.hyphenation.do_not_hyphenate_caps);
        let off = DocumentSettings::from_xml(
            r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:autoHyphenation w:val="0"/></w:settings>"#,
        )
        .unwrap();
        assert!(!off.hyphenation.auto);
    }

    #[test]
    fn parses_mirror_margins_toggle() {
        let on_xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:mirrorMargins/>
        </w:settings>
        "#;
        let settings = DocumentSettings::from_xml(on_xml).expect("parse settings");
        assert!(settings.mirror_margins);

        let off_xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:mirrorMargins w:val="0"></w:mirrorMargins>
        </w:settings>
        "#;
        let settings = DocumentSettings::from_xml(off_xml).expect("parse settings");
        assert!(!settings.mirror_margins);
    }

    #[test]
    fn parses_document_variables_with_quoted_field_names_and_xml_entities() {
        let xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:docVars>
            <w:docVar w:name="Release Channel" w:val="Beta &amp; Preview"/>
            <w:docVar w:name="BuildNumber" w:val="2026.07"/>
          </w:docVars>
        </w:settings>
        "#;

        let settings = DocumentSettings::from_xml(xml).expect("parse settings");
        assert_eq!(
            settings
                .document_variables
                .get("release channel")
                .map(String::as_str),
            Some("Beta & Preview")
        );
        assert_eq!(
            settings
                .document_variables
                .get("buildnumber")
                .map(String::as_str),
            Some("2026.07")
        );
    }

    #[test]
    fn parses_word_layout_flags() {
        let xml = r#"
        <w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:kinsoku w:val="0"/>
          <w:wordWrap/>
          <w:overflowPunct w:val="false"/>
          <w:topLinePunct/>
        </w:settings>
        "#;

        let settings = DocumentSettings::from_xml(xml).expect("parse settings");
        assert_eq!(settings.kinsoku, Some(false));
        assert_eq!(settings.word_wrap, Some(true));
        assert_eq!(settings.overflow_punctuation, Some(false));
        assert_eq!(settings.top_line_punctuation, Some(true));

        let converted = settings.to_line_breaking_settings();
        assert_eq!(converted.kinsoku, Some(false));
        assert_eq!(converted.word_wrap, Some(true));
        assert_eq!(converted.overflow_punctuation, Some(false));
        assert_eq!(converted.top_line_punctuation, Some(true));
    }
}
