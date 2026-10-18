use crate::document::{FontHint, ParagraphProperties, RunFonts, RunProperties};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::rc::Rc;

type ParseResult<T> = Result<T, Box<dyn std::error::Error>>;

/// Parsed representation of `word/numbering.xml`. Stores abstract numbering
/// definitions and concrete numbering instances that paragraphs refer to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberingPart {
    pub abstract_nums: HashMap<String, AbstractNum>,
    pub numbering_instances: HashMap<String, NumberingInstance>,
}

impl Default for NumberingPart {
    fn default() -> Self {
        Self::new()
    }
}

/// Describes a multiline numbering definition that may contain up to nine
/// levels, each with its own formatting and indentation rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbstractNum {
    pub id: String,
    pub levels: Vec<NumberingLevel>,
    /// The built-in gallery this definition was created from in this session,
    /// so applying the same gallery again continues the same list. Word keeps
    /// no such marker in `numbering.xml`, so a reopened document has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<MultilevelListPreset>,
}

/// Stores the configuration for a single numbering level, including the
/// textual template and any optional paragraph/run overrides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberingLevel {
    pub ilvl: u32,
    pub start: Option<u32>,
    pub restart: Option<u32>,
    pub num_fmt: Option<String>,
    pub lvl_text: Option<String>,
    pub lvl_jc: Option<String>,
    pub suffix: Option<String>,
    pub indent: Option<NumberingIndent>,
    pub paragraph_style: Option<String>,
    pub paragraph_props: Option<ParagraphProperties>,
    pub run_props: Option<RunProperties>,
    /// Language of the numbering label from `w:lvl/w:rPr/w:lang/@w:val`.
    /// Language-dependent formats must not guess from the document body.
    pub language: Option<String>,
}

/// Indentation offsets used when the numbering definition contributes its own
/// layout (common for bullet lists).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberingIndent {
    pub left: Option<f32>,
    pub hanging: Option<f32>,
}

/// Associates a numbering instance with its referenced abstract definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberingInstance {
    pub id: String,
    pub abstract_num_id: String,
    pub overrides: HashMap<u32, LevelOverride>,
}

/// Built-in nine-level list galleries shared by the editor integrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MultilevelListPreset {
    DecimalHierarchy,
    ChineseOfficial,
    BulletHierarchy,
}

impl MultilevelListPreset {
    pub fn key(self) -> &'static str {
        match self {
            Self::DecimalHierarchy => "decimalHierarchy",
            Self::ChineseOfficial => "chineseOfficial",
            Self::BulletHierarchy => "bulletHierarchy",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LevelOverride {
    pub start: Option<u32>,
}

impl NumberingPart {
    /// Creates an empty numbering part. Useful when documents do not contain
    /// any numbering and we need a neutral default.
    pub fn new() -> Self {
        Self {
            abstract_nums: HashMap::new(),
            numbering_instances: HashMap::new(),
        }
    }

    /// Parses an XML string containing numbering definitions and returns the
    /// populated [`NumberingPart`].
    pub fn from_xml(xml: &str) -> ParseResult<Self> {
        let mut part = Self::new();
        part.parse(xml)?;
        Ok(part)
    }

    /// Parses numbering information and mutates the existing structure.
    pub fn parse(&mut self, xml: &str) -> ParseResult<()> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut buf = Vec::new();

        let mut current_abstract: Option<AbstractNum> = None;
        let mut current_level: Option<NumberingLevel> = None;
        let mut current_num: Option<NumberingInstance> = None;
        let mut current_override_level: Option<u32> = None;
        let mut in_level_paragraph_props = false;
        let mut in_level_run_props = false;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    self.handle_start_event(
                        e,
                        &mut current_abstract,
                        &mut current_level,
                        &mut current_num,
                        &mut current_override_level,
                        &mut in_level_paragraph_props,
                        &mut in_level_run_props,
                    )?;
                }
                Ok(Event::Empty(ref e)) => {
                    self.handle_start_event(
                        e,
                        &mut current_abstract,
                        &mut current_level,
                        &mut current_num,
                        &mut current_override_level,
                        &mut in_level_paragraph_props,
                        &mut in_level_run_props,
                    )?;
                    self.handle_end_event(
                        e.name().as_ref(),
                        &mut current_abstract,
                        &mut current_level,
                        &mut current_num,
                        &mut current_override_level,
                        &mut in_level_paragraph_props,
                        &mut in_level_run_props,
                    );
                }
                Ok(Event::End(ref e)) => {
                    self.handle_end_event(
                        e.name().as_ref(),
                        &mut current_abstract,
                        &mut current_level,
                        &mut current_num,
                        &mut current_override_level,
                        &mut in_level_paragraph_props,
                        &mut in_level_run_props,
                    );
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    return Err(format!("Numbering XML parsing error: {}", e).into());
                }
                _ => {}
            }

            buf.clear();
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_start_event(
        &mut self,
        e: &BytesStart,
        current_abstract: &mut Option<AbstractNum>,
        current_level: &mut Option<NumberingLevel>,
        current_num: &mut Option<NumberingInstance>,
        current_override_level: &mut Option<u32>,
        in_level_paragraph_props: &mut bool,
        in_level_run_props: &mut bool,
    ) -> ParseResult<()> {
        match e.name().as_ref() {
            b"w:abstractNum" => {
                if let Some(id) = attr_value(e, b"w:abstractNumId") {
                    *current_abstract = Some(AbstractNum {
                        id,
                        levels: Vec::new(),
                        preset: None,
                    });
                }
            }
            b"w:lvl" => {
                if let Some(ilvl) = attr_value(e, b"w:ilvl") {
                    let level = NumberingLevel {
                        ilvl: ilvl.parse().unwrap_or(0),
                        start: None,
                        restart: None,
                        num_fmt: None,
                        lvl_text: None,
                        lvl_jc: None,
                        suffix: None,
                        indent: None,
                        paragraph_style: None,
                        paragraph_props: None,
                        run_props: None,
                        language: None,
                    };
                    *current_level = Some(level);
                }
            }
            b"w:start" => {
                if let Some(level) = current_level.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        level.start = val.parse().ok();
                    }
                }
            }
            b"w:lvlRestart" => {
                if let Some(level) = current_level.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        level.restart = val.parse().ok();
                    }
                }
            }
            b"w:numFmt" => {
                if let Some(level) = current_level.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        level.num_fmt = Some(val);
                    }
                }
            }
            b"w:lvlText" => {
                if let Some(level) = current_level.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        level.lvl_text = Some(val);
                    }
                }
            }
            b"w:lvlJc" => {
                if let Some(level) = current_level.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        level.lvl_jc = Some(val);
                    }
                }
            }
            b"w:suff" => {
                if let Some(level) = current_level.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        level.suffix = Some(val);
                    }
                }
            }
            b"w:pStyle" => {
                if let Some(level) = current_level.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        level.paragraph_style = Some(val);
                    }
                }
            }
            b"w:pPr" => {
                if current_level.is_some() {
                    *in_level_paragraph_props = true;
                }
            }
            b"w:ind" => {
                if *in_level_paragraph_props {
                    if let Some(level) = current_level.as_mut() {
                        let mut indent = level.indent.clone().unwrap_or(NumberingIndent {
                            left: None,
                            hanging: None,
                        });

                        if let Some(val) = attr_value(e, b"w:left") {
                            if let Ok(parsed) = val.parse::<f32>() {
                                indent.left = Some(parsed / 20.0);
                            }
                        }

                        if let Some(val) = attr_value(e, b"w:hanging") {
                            if let Ok(parsed) = val.parse::<f32>() {
                                indent.hanging = Some(parsed / 20.0);
                            }
                        }

                        level.indent = Some(indent);
                    }
                }
            }
            b"w:rPr" => {
                if current_level.is_some() {
                    *in_level_run_props = true;
                }
            }
            b"w:b" => {
                if *in_level_run_props {
                    if let Some(level) = current_level.as_mut() {
                        let props = ensure_run_props(level);
                        props.bold = Some(true);
                    }
                }
            }
            b"w:i" => {
                if *in_level_run_props {
                    if let Some(level) = current_level.as_mut() {
                        let props = ensure_run_props(level);
                        props.italic = Some(true);
                    }
                }
            }
            b"w:u" => {
                if *in_level_run_props {
                    if let Some(level) = current_level.as_mut() {
                        let props = ensure_run_props(level);
                        props.underline = Some(true);
                    }
                }
            }
            b"w:color" => {
                if *in_level_run_props {
                    if let Some(level) = current_level.as_mut() {
                        if let Some(val) = attr_value(e, b"w:val") {
                            let props = ensure_run_props(level);
                            props.color = Some(Rc::from(format!("#{}", val)));
                        }
                    }
                }
            }
            b"w:sz" => {
                if *in_level_run_props {
                    if let Some(level) = current_level.as_mut() {
                        if let Some(val) = attr_value(e, b"w:val") {
                            if let Ok(parsed) = val.parse::<f32>() {
                                let props = ensure_run_props(level);
                                props.font_size = Some(parsed / 2.0);
                            }
                        }
                    }
                }
            }
            b"w:spacing" if *in_level_run_props => {
                if let (Some(level), Some(value)) =
                    (current_level.as_mut(), attr_value(e, b"w:val"))
                {
                    if let Ok(twips) = value.parse::<f32>() {
                        if twips.is_finite() {
                            ensure_run_props(level).char_spacing = Some(twips / 20.0);
                        }
                    }
                }
            }
            b"w:rFonts" => {
                if *in_level_run_props {
                    if let Some(level) = current_level.as_mut() {
                        let props = ensure_run_props(level);
                        let fonts = props.fonts.get_or_insert_with(RunFonts::default);
                        for attr in e.attributes().flatten() {
                            let key = attr.key.as_ref();
                            let value = attr
                                .unescape_value()
                                .map(|value| value.into_owned())
                                .unwrap_or_else(|_| {
                                    String::from_utf8_lossy(&attr.value).to_string()
                                });
                            match key {
                                b"w:ascii" => {
                                    fonts.ascii = Some(Rc::from(value));
                                }
                                b"w:hAnsi" => {
                                    fonts.h_ansi = Some(Rc::from(value));
                                }
                                b"w:eastAsia" => {
                                    fonts.east_asia = Some(Rc::from(value));
                                }
                                b"w:cs" => {
                                    fonts.complex = Some(Rc::from(value));
                                }
                                b"w:asciiTheme" => {
                                    fonts.ascii_theme = Some(Rc::from(value));
                                }
                                b"w:hAnsiTheme" => {
                                    fonts.h_ansi_theme = Some(Rc::from(value));
                                }
                                b"w:eastAsiaTheme" => {
                                    fonts.east_asia_theme = Some(Rc::from(value));
                                }
                                b"w:cstheme" => {
                                    fonts.complex_theme = Some(Rc::from(value));
                                }
                                b"w:hint" => {
                                    fonts.hint = FontHint::from_xml_hint(&value);
                                }
                                _ => {}
                            }
                        }
                        fonts.drop_names_shadowed_by_themes();
                        if props.font_family.is_none() {
                            if let Some(family) = fonts.preferred_latin() {
                                props.font_family = Some(Rc::from(family));
                            }
                        }
                    }
                }
            }
            b"w:lang" => {
                if *in_level_run_props {
                    if let (Some(level), Some(language)) =
                        (current_level.as_mut(), attr_value(e, b"w:val"))
                    {
                        level.language = Some(language);
                    }
                }
            }
            b"w:num" => {
                if let Some(id) = attr_value(e, b"w:numId") {
                    *current_num = Some(NumberingInstance {
                        id,
                        abstract_num_id: String::new(),
                        overrides: HashMap::new(),
                    });
                }
            }
            b"w:abstractNumId" => {
                if let Some(num) = current_num.as_mut() {
                    if let Some(val) = attr_value(e, b"w:val") {
                        num.abstract_num_id = val;
                    }
                }
            }
            b"w:lvlOverride" => {
                if let Some(ilvl) = attr_value(e, b"w:ilvl") {
                    *current_override_level = ilvl.parse::<u32>().ok();
                }
            }
            b"w:startOverride" => {
                if let (Some(level), Some(num)) = (*current_override_level, current_num.as_mut()) {
                    if let Some(val) = attr_value(e, b"w:val") {
                        let override_level = num.overrides.entry(level).or_default();
                        override_level.start = val.parse().ok();
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_end_event(
        &mut self,
        name: &[u8],
        current_abstract: &mut Option<AbstractNum>,
        current_level: &mut Option<NumberingLevel>,
        current_num: &mut Option<NumberingInstance>,
        current_override_level: &mut Option<u32>,
        in_level_paragraph_props: &mut bool,
        in_level_run_props: &mut bool,
    ) {
        match name {
            b"w:lvl" => {
                if let Some(level) = current_level.take() {
                    if let Some(abs) = current_abstract.as_mut() {
                        abs.levels.push(level);
                    }
                }
            }
            b"w:abstractNum" => {
                if let Some(abs) = current_abstract.take() {
                    if !abs.id.is_empty() {
                        self.abstract_nums.insert(abs.id.clone(), abs);
                    }
                }
            }
            b"w:num" => {
                if let Some(num) = current_num.take() {
                    if !num.id.is_empty() && !num.abstract_num_id.is_empty() {
                        self.numbering_instances.insert(num.id.clone(), num);
                    }
                }
            }
            b"w:lvlOverride" => {
                *current_override_level = None;
            }
            b"w:pPr" => {
                *in_level_paragraph_props = false;
            }
            b"w:rPr" => {
                *in_level_run_props = false;
            }
            _ => {}
        }
    }

    /// Finds an existing numbering instance whose level-0 format matches
    /// `num_fmt` (e.g. `"bullet"` or `"decimal"`).  Returns the instance ID.
    pub fn find_instance_by_format(&self, num_fmt: &str) -> Option<&str> {
        for (id, instance) in &self.numbering_instances {
            if let Some(abstract_num) = self.abstract_nums.get(&instance.abstract_num_id) {
                if let Some(level) = abstract_num.levels.iter().find(|l| l.ilvl == 0) {
                    if level.num_fmt.as_deref() == Some(num_fmt) {
                        return Some(id.as_str());
                    }
                }
            }
        }
        None
    }

    /// Returns an existing bullet list numbering instance ID, or creates one
    /// and returns its ID.
    pub fn ensure_bullet_instance(&mut self) -> String {
        if let Some(id) = self.find_instance_by_format("bullet") {
            return id.to_string();
        }
        self.create_builtin_instance("bullet")
    }

    /// Returns an existing numbered list instance ID, or creates one and
    /// returns its ID.
    pub fn ensure_decimal_instance(&mut self) -> String {
        if let Some(id) = self.find_instance_by_format("decimal") {
            return id.to_string();
        }
        self.create_builtin_instance("decimal")
    }

    /// Returns the stable instance for a built-in nine-level list gallery,
    /// creating the definition when this document has not used it before.
    pub fn ensure_multilevel_instance(&mut self, preset: MultilevelListPreset) -> String {
        let existing = self
            .abstract_nums
            .values()
            .find(|definition| definition.preset == Some(preset))
            .map(|definition| definition.id.clone());
        if let Some(abstract_id) = &existing {
            if let Some((id, _)) = self
                .numbering_instances
                .iter()
                .find(|(_, instance)| &instance.abstract_num_id == abstract_id)
            {
                return id.clone();
            }
        }

        // `w:abstractNumId` is an integer (ST_DecimalNumber); Word refuses a
        // numbering part whose ids are not.
        let abstract_id = existing.unwrap_or_else(|| {
            let id = self.next_abstract_num_id();
            self.abstract_nums.insert(
                id.clone(),
                AbstractNum {
                    id: id.clone(),
                    levels: multilevel_preset_levels(preset),
                    preset: Some(preset),
                },
            );
            id
        });
        let instance_id = self.next_instance_id();
        self.numbering_instances.insert(
            instance_id.clone(),
            NumberingInstance {
                id: instance_id.clone(),
                abstract_num_id: abstract_id,
                overrides: HashMap::new(),
            },
        );
        instance_id
    }

    /// Creates a distinct abstract definition and numbering instance for a
    /// custom multilevel list. Custom definitions are intentionally not
    /// deduplicated: Word's "Define New List" command creates a new list whose
    /// later restart/continue behavior must remain independent from an older
    /// definition with the same visible formatting.
    pub fn create_custom_multilevel_instance(&mut self, levels: Vec<NumberingLevel>) -> String {
        let abstract_id = self.next_abstract_num_id();
        let instance_id = self.next_instance_id();
        self.abstract_nums.insert(
            abstract_id.clone(),
            AbstractNum {
                id: abstract_id.clone(),
                levels,
                preset: None,
            },
        );
        self.numbering_instances.insert(
            instance_id.clone(),
            NumberingInstance {
                id: instance_id.clone(),
                abstract_num_id: abstract_id,
                overrides: HashMap::new(),
            },
        );
        instance_id
    }

    /// Creates a minimal abstract numbering + instance for the given format.
    fn create_builtin_instance(&mut self, num_fmt: &str) -> String {
        let abstract_id = self.next_abstract_num_id();
        let instance_id = self.next_instance_id();

        let (lvl_text, run_props) = if num_fmt == "bullet" {
            (
                Some("\u{2022}".to_string()), // bullet character
                Some(RunProperties {
                    fonts: Some(RunFonts {
                        ascii: Some(Rc::from("Symbol")),
                        h_ansi: Some(Rc::from("Symbol")),
                        hint: Some(FontHint::Default),
                        ..RunFonts::default()
                    }),
                    ..RunProperties::default()
                }),
            )
        } else {
            (Some("%1.".to_string()), None)
        };

        let abstract_num = AbstractNum {
            id: abstract_id.clone(),
            levels: vec![NumberingLevel {
                ilvl: 0,
                start: Some(1),
                restart: None,
                num_fmt: Some(num_fmt.to_string()),
                lvl_text,
                lvl_jc: Some("left".to_string()),
                suffix: Some("tab".to_string()),
                indent: Some(NumberingIndent {
                    left: Some(36.0),
                    hanging: Some(18.0),
                }),
                paragraph_style: None,
                paragraph_props: None,
                run_props,
                language: None,
            }],
            preset: None,
        };

        let instance = NumberingInstance {
            id: instance_id.clone(),
            abstract_num_id: abstract_id.clone(),
            overrides: HashMap::new(),
        };

        self.abstract_nums.insert(abstract_id, abstract_num);
        self.numbering_instances
            .insert(instance_id.clone(), instance);
        instance_id
    }

    fn next_instance_id(&self) -> String {
        let max_numeric = self
            .numbering_instances
            .keys()
            .filter_map(|id| id.parse::<u32>().ok())
            .max()
            .unwrap_or(999);
        let mut candidate = max_numeric.saturating_add(1).max(1000);
        while self
            .numbering_instances
            .contains_key(&candidate.to_string())
        {
            candidate = candidate.saturating_add(1);
        }
        candidate.to_string()
    }

    fn next_abstract_num_id(&self) -> String {
        let max_numeric = self
            .abstract_nums
            .keys()
            .filter_map(|id| id.parse::<u32>().ok())
            .max()
            .unwrap_or(999);
        let mut candidate = max_numeric.saturating_add(1).max(1000);
        while self.abstract_nums.contains_key(&candidate.to_string()) {
            candidate = candidate.saturating_add(1);
        }
        candidate.to_string()
    }

    /// Retrieves the numbering level configuration for a concrete numbering
    /// instance. Returns `None` when the instance or level is missing.
    pub fn get_level_info(&self, num_id: &str, level: u32) -> Option<&NumberingLevel> {
        let instance = self.numbering_instances.get(num_id)?;
        let abstract_num = self.abstract_nums.get(&instance.abstract_num_id)?;

        abstract_num.levels.iter().find(|l| l.ilvl == level)
    }

    pub fn has_instance(&self, num_id: &str) -> bool {
        self.numbering_instances.contains_key(num_id)
    }

    pub fn abstract_num_id(&self, num_id: &str) -> Option<&str> {
        self.numbering_instances
            .get(num_id)
            .map(|instance| instance.abstract_num_id.as_str())
    }

    pub fn has_start_override(&self, num_id: &str, level: u32) -> bool {
        self.numbering_instances
            .get(num_id)
            .and_then(|instance| instance.overrides.get(&level))
            .and_then(|override_level| override_level.start)
            .is_some()
    }

    pub fn start_value(&self, num_id: &str, level: u32) -> Option<u32> {
        let instance = self.numbering_instances.get(num_id)?;
        if let Some(override_level) = instance.overrides.get(&level) {
            if override_level.start.is_some() {
                return override_level.start;
            }
        }
        self.abstract_nums
            .get(&instance.abstract_num_id)?
            .levels
            .iter()
            .find(|lvl| lvl.ilvl == level)
            .and_then(|lvl| lvl.start)
    }

    /// Formats a numbering text according to the level template (e.g. `%1.` or
    /// `%1.%2.`). The method depends on the [`NumberingCounter`] to provide
    /// per-level counter values.
    pub fn format_number(&self, num_id: &str, level: u32, counters: &NumberingCounter) -> String {
        self.format_number_with_offsets(num_id, level, counters, None)
    }

    pub fn format_number_with_offsets(
        &self,
        num_id: &str,
        level: u32,
        counters: &NumberingCounter,
        offsets: Option<&HashMap<String, Vec<u32>>>,
    ) -> String {
        if !self.has_instance(num_id) {
            return String::new();
        }

        // `none` suppresses the numbering label, not merely the numeric part
        // substituted into `lvlText`. Returning before template expansion keeps
        // punctuation such as `%1.` from leaking as a standalone period.
        if self
            .get_level_info(num_id, level)
            .and_then(|info| info.num_fmt.as_deref())
            == Some("none")
        {
            return String::new();
        }

        let default_template = "%1.".to_string();

        let template = self
            .get_level_info(num_id, level)
            .and_then(|info| info.lvl_text.clone())
            .unwrap_or(default_template);

        let mut result = String::new();
        let mut chars = template.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '%' {
                let mut digits = String::new();
                while let Some(next) = chars.peek() {
                    if next.is_ascii_digit() {
                        digits.push(*next);
                        chars.next();
                    } else {
                        break;
                    }
                }

                if digits.is_empty() {
                    result.push('%');
                    continue;
                }

                if let Ok(idx) = digits.parse::<u32>() {
                    if idx >= 1 {
                        let placeholder_level = idx - 1;
                        let mut value = counters.get_counter(num_id, placeholder_level);
                        if counters.counter_value(num_id, placeholder_level).is_none() {
                            if let Some(info) = self.get_level_info(num_id, placeholder_level) {
                                value = info.start.unwrap_or(1);
                            }
                        }
                        if let Some(offsets) = offsets {
                            if let Some(level_offsets) = offsets.get(num_id) {
                                if let Some(offset) = level_offsets.get(placeholder_level as usize)
                                {
                                    if *offset > 0 && value > 0 {
                                        value = value.saturating_sub(*offset);
                                        if value == 0 {
                                            value = 1;
                                        }
                                    }
                                }
                            }
                        }
                        let formatted = self.format_counter_value(num_id, placeholder_level, value);
                        result.push_str(&formatted);
                        continue;
                    }
                }

                // Fallback to literal if parsing failed
                result.push('%');
                result.push_str(&digits);
            } else {
                result.push(ch);
            }
        }

        if let Some(level_info) = self.get_level_info(num_id, level) {
            if let Some(suffix) = level_info.suffix.as_deref() {
                if suffix == "tab" || suffix == "space" {
                    result.push(' ');
                }
            }
        }

        result
    }

    fn format_counter_value(&self, num_id: &str, level: u32, value: u32) -> String {
        let format = self
            .get_level_info(num_id, level)
            .and_then(|info| info.num_fmt.clone())
            .unwrap_or_else(|| "decimal".to_string());

        // Decimal formats represent zero directly. Preserve the existing
        // one-based fallback for alphabetic and other numbering systems.
        let value = if matches!(
            format.as_str(),
            "decimal"
                | "decimalHalfWidth"
                | "decimalFullWidth"
                | "decimalFullWidth2"
                | "decimalZero"
        ) {
            value
        } else {
            value.max(1)
        };
        let language = self
            .get_level_info(num_id, level)
            .and_then(|info| info.language.as_deref());

        format_number_value(&format, value, language)
    }
}

/// Formats `value` with an `ST_NumberFormat` token (ECMA-376 Part 1
/// §17.18.59). List levels, page numbers (`w:pgNumType/@w:fmt`) and note
/// markers (`w:numFmt` in `w:footnotePr`/`w:endnotePr`) all name their counter
/// style with this one simple type, so they share this formatter. Tokens match
/// case-insensitively: section and note formats are kept lowercased by the
/// parser. Unknown tokens fall back to decimal.
///
/// Zero has no representation in most systems; it stays a decimal `0` here
/// and list levels clamp to one before calling.
pub fn format_number_value(format: &str, value: u32, language: Option<&str>) -> String {
    let format = canonical_number_format(format).unwrap_or(format);
    if value == 0
        && !matches!(
            format,
            "decimal"
                | "decimalHalfWidth"
                | "decimalFullWidth"
                | "decimalFullWidth2"
                | "decimalZero"
                | "hindiNumbers"
                | "thaiNumbers"
                | "numberInDash"
                | "hex"
                | "none"
                | "bullet"
        )
    {
        return value.to_string();
    }
    match format {
        "decimal" => value.to_string(),
        "upperRoman" => to_roman(value, true),
        "lowerRoman" => to_roman(value, false),
        "upperLetter" => to_letter(value, true),
        "lowerLetter" => to_letter(value, false),
        "ordinal" if language.is_some_and(is_english_language) => to_english_ordinal(value),
        "cardinalText" if language.is_some_and(is_english_language) => to_english_cardinal(value),
        "ordinalText" if language.is_some_and(is_english_language) => {
            to_english_ordinal_text(value)
        }
        "hex" => to_hex(value),
        // `target/contract-numbering-word-trace.json` records Word COM
        // `ListString` as `第１条　` for a `decimalFullWidth` heading.
        // Keeping this formatter explicit lets TRACE reports distinguish
        // true numbering interpretation bugs from later layout/metric drift.
        "decimalFullWidth" | "decimalFullWidth2" => to_full_width_decimal(value),
        // Keep this token explicit even though Rust's decimal output is
        // already single-byte ASCII. Otherwise a future generic fallback
        // could silently change a format whose width is layout-significant.
        "decimalHalfWidth" => value.to_string(),
        // `decimalZero` is a *minimum* width of two, not a fixed-width
        // counter: Word emits 01..09 and then continues with 10, 11, ... .
        // This distinction affects the numbering label advance and thus the
        // first-line text origin for single-digit list items.
        "decimalZero" => format!("{value:02}"),
        // Word follows the ST_NumberFormat examples: the unsuffixed token
        // uses half-width Katakana and the FullWidth token uses ordinary
        // full-width Katakana. This intentionally follows observed Word
        // interoperability rather than the contradictory "Hiragana" label
        // attached to aiueo in the schema prose.
        "aiueo" => to_kana_sequence(value, AIUEO_KATAKANA_HALF_WIDTH),
        "aiueoFullWidth" => to_kana_sequence(value, AIUEO_KATAKANA_FULL_WIDTH),
        "iroha" => to_kana_sequence(value, IROHA_KATAKANA_HALF_WIDTH),
        "irohaFullWidth" => to_kana_sequence(value, IROHA_KATAKANA_FULL_WIDTH),
        // Both formats are positional digit sequences (10 -> 一〇), unlike
        // japaneseCounting (10 -> 十). Microsoft notes that Word does not
        // apply the circle suggested by old ideographDigital prose.
        "japaneseDigitalTenThousand" | "ideographDigital" => to_japanese_positional_digits(value),
        "japaneseCounting" => to_japanese_counting(value, JapaneseKanjiStyle::Standard),
        "chineseCountingThousand" => to_japanese_counting(value, JapaneseKanjiStyle::Standard),
        "japaneseLegal" => to_japanese_counting(value, JapaneseKanjiStyle::Legal),
        // ST_NumberFormat defines dedicated Unicode enclosed glyphs only
        // through 20 (or 10 for the Chinese circle variant). Word permits
        // ordinary decimal output after that range, which is preferable to
        // synthesizing a visually different combining enclosure.
        "decimalEnclosedCircle" => to_enclosed_decimal(value, '①', 20),
        "decimalEnclosedFullstop" => to_enclosed_decimal(value, '⒈', 20),
        "decimalEnclosedParen" => to_enclosed_decimal(value, '⑴', 20),
        "decimalEnclosedCircleChinese" => to_enclosed_decimal(value, '①', 10),
        "ideographEnclosedCircle" => to_enclosed_ideograph(value),
        // These OOXML formats preserve the decimal place-value system and
        // replace each ASCII digit with the corresponding native digit.
        // Formatting the whole value first also handles embedded zeroes.
        "hindiNumbers" => to_decimal_digit_set(value, '०'),
        "thaiNumbers" => to_decimal_digit_set(value, '๐'),
        "numberInDash" => format!("- {value} -"),
        "chicago" => to_chicago_symbols(value),
        // Ten heavenly stems, twelve earthly branches and their sixty-term
        // sexagenary cycle. These are closed sets; Word wraps round them.
        "ideographTraditional" => cycle_char(value, HEAVENLY_STEMS),
        "ideographZodiac" => cycle_char(value, EARTHLY_BRANCHES),
        "ideographZodiacTraditional" => to_sexagenary(value),
        "taiwaneseDigital" => to_japanese_positional_digits(value),
        "chineseCounting" | "taiwaneseCounting" | "taiwaneseCountingThousand" => {
            to_chinese_counting(value, ChineseNumerals::COUNTING)
        }
        "chineseLegalSimplified" => to_chinese_counting(value, ChineseNumerals::LEGAL_SIMPLIFIED),
        "ideographLegalTraditional" => {
            to_chinese_counting(value, ChineseNumerals::LEGAL_TRADITIONAL)
        }
        "koreanDigital" => to_decimal_digits(value, KOREAN_DIGITS),
        "koreanCounting" => to_korean_counting(value),
        "koreanLegal" => to_korean_native(value),
        "ganada" => cycle_char(value, GANADA),
        "chosung" => cycle_char(value, CHOSUNG),
        "russianLower" => to_repeated_letter(value, RUSSIAN_LOWER),
        "russianUpper" => to_repeated_letter(value, RUSSIAN_UPPER),
        "hebrew1" => to_hebrew_numeral(value),
        "hebrew2" => to_repeated_letter(value, HEBREW_ALPHABET),
        "arabicAlpha" => to_repeated_letter(value, ARABIC_ALPHA),
        "arabicAbjad" => to_repeated_letter(value, ARABIC_ABJAD),
        "none" => String::new(),
        "bullet" => "•".to_string(),
        _ => value.to_string(),
    }
}

/// Every `ST_NumberFormat` token this module formats, in schema spelling.
const NUMBER_FORMAT_TOKENS: &[&str] = &[
    "decimal",
    "upperRoman",
    "lowerRoman",
    "upperLetter",
    "lowerLetter",
    "ordinal",
    "cardinalText",
    "ordinalText",
    "hex",
    "chicago",
    "ideographDigital",
    "japaneseCounting",
    "aiueo",
    "iroha",
    "decimalFullWidth",
    "decimalHalfWidth",
    "japaneseLegal",
    "japaneseDigitalTenThousand",
    "decimalEnclosedCircle",
    "decimalFullWidth2",
    "aiueoFullWidth",
    "irohaFullWidth",
    "decimalZero",
    "bullet",
    "ganada",
    "chosung",
    "decimalEnclosedFullstop",
    "decimalEnclosedParen",
    "decimalEnclosedCircleChinese",
    "ideographEnclosedCircle",
    "ideographTraditional",
    "ideographZodiac",
    "ideographZodiacTraditional",
    "taiwaneseCounting",
    "ideographLegalTraditional",
    "taiwaneseCountingThousand",
    "taiwaneseDigital",
    "chineseCounting",
    "chineseLegalSimplified",
    "chineseCountingThousand",
    "koreanDigital",
    "koreanCounting",
    "koreanLegal",
    "russianLower",
    "russianUpper",
    "none",
    "numberInDash",
    "hebrew1",
    "hebrew2",
    "arabicAlpha",
    "arabicAbjad",
    "hindiNumbers",
    "thaiNumbers",
];

/// The schema spelling of an `ST_NumberFormat` token this module formats,
/// matched case-insensitively (`"upperroman"` → `"upperRoman"`).
pub fn canonical_number_format(format: &str) -> Option<&'static str> {
    NUMBER_FORMAT_TOKENS
        .iter()
        .find(|token| token.eq_ignore_ascii_case(format))
        .copied()
}

/// Chicago Manual of Style note symbols: `*`, `†`, `‡`, `§`, then the same
/// four doubled, tripled, and so on.
fn to_chicago_symbols(value: u32) -> String {
    const SYMBOLS: [char; 4] = ['*', '†', '‡', '§'];
    to_repeated_symbol(value, &SYMBOLS)
}

const HEAVENLY_STEMS: &str = "甲乙丙丁戊己庚辛壬癸";
const EARTHLY_BRANCHES: &str = "子丑寅卯辰巳午未申酉戌亥";

/// Korean consonant-plus-a syllables, the order Hangul dictionaries use.
const GANADA: &str = "가나다라마바사아자차카타파하";
/// The fourteen basic initial consonants (jamo) in the same order.
const CHOSUNG: &str = "ㄱㄴㄷㄹㅁㅂㅅㅇㅈㅊㅋㅌㅍㅎ";

/// The Russian alphabet without ё, й, ъ, ы and ь, which are not used as
/// list labels.
const RUSSIAN_LOWER: &[char] = &[
    'а', 'б', 'в', 'г', 'д', 'е', 'ж', 'з', 'и', 'к', 'л', 'м', 'н', 'о', 'п', 'р', 'с', 'т', 'у',
    'ф', 'х', 'ц', 'ч', 'ш', 'щ', 'э', 'ю', 'я',
];
const RUSSIAN_UPPER: &[char] = &[
    'А', 'Б', 'В', 'Г', 'Д', 'Е', 'Ж', 'З', 'И', 'К', 'Л', 'М', 'Н', 'О', 'П', 'Р', 'С', 'Т', 'У',
    'Ф', 'Х', 'Ц', 'Ч', 'Ш', 'Щ', 'Э', 'Ю', 'Я',
];
const HEBREW_ALPHABET: &[char] = &[
    'א', 'ב', 'ג', 'ד', 'ה', 'ו', 'ז', 'ח', 'ט', 'י', 'כ', 'ל', 'מ', 'נ', 'ס', 'ע', 'פ', 'צ', 'ק',
    'ר', 'ש', 'ת',
];
/// Hijāʾī order (letters grouped by shape).
const ARABIC_ALPHA: &[char] = &[
    'أ', 'ب', 'ت', 'ث', 'ج', 'ح', 'خ', 'د', 'ذ', 'ر', 'ز', 'س', 'ش', 'ص', 'ض', 'ط', 'ظ', 'ع', 'غ',
    'ف', 'ق', 'ك', 'ل', 'م', 'ن', 'ه', 'و', 'ي',
];
/// Abjadī order (the letters' traditional numeric values 1, 2, 3, …).
const ARABIC_ABJAD: &[char] = &[
    'أ', 'ب', 'ج', 'د', 'ه', 'و', 'ز', 'ح', 'ط', 'ي', 'ك', 'ل', 'م', 'ن', 'س', 'ع', 'ف', 'ص', 'ق',
    'ر', 'ش', 'ت', 'ث', 'خ', 'ذ', 'ض', 'ظ', 'غ',
];
const KOREAN_DIGITS: [char; 10] = ['영', '일', '이', '삼', '사', '오', '육', '칠', '팔', '구'];

fn cycle_char(value: u32, alphabet: &str) -> String {
    let len = alphabet.chars().count() as u32;
    alphabet
        .chars()
        .nth(((value.max(1) - 1) % len) as usize)
        .map(String::from)
        .unwrap_or_default()
}

/// `甲子`, `乙丑`, … `癸亥`: the stem and branch advance together, so the
/// cycle repeats after sixty terms.
fn to_sexagenary(value: u32) -> String {
    let index = (value.max(1) - 1) % 60;
    let mut label = cycle_char(index % 10 + 1, HEAVENLY_STEMS);
    label.push_str(&cycle_char(index % 12 + 1, EARTHLY_BRANCHES));
    label
}

/// Longest label the repeating formats build before falling back to decimal;
/// Word's own lettered lists stop well short of this.
const MAX_REPEATED_LABEL: u32 = 40;

/// Word's alphabetic rollover: after the last letter the first comes back
/// doubled, then the second doubled (`z`, `aa`, `bb`, …), not the
/// spreadsheet-style `aa`, `ab`.
fn to_repeated_letter(value: u32, alphabet: &[char]) -> String {
    to_repeated_symbol(value, alphabet)
}

fn to_repeated_symbol(value: u32, alphabet: &[char]) -> String {
    let len = alphabet.len() as u32;
    let index = value.max(1) - 1;
    let times = index / len + 1;
    if times > MAX_REPEATED_LABEL {
        return value.to_string();
    }
    std::iter::repeat_n(alphabet[(index % len) as usize], times as usize).collect()
}

fn to_decimal_digits(value: u32, digits: [char; 10]) -> String {
    value
        .to_string()
        .bytes()
        .map(|digit| digits[(digit - b'0') as usize])
        .collect()
}

/// Digits, small units and large units of one Chinese counting style.
struct ChineseNumerals {
    /// Index 0 is the zero written between non-zero places (一百〇一).
    digits: [char; 10],
    /// 十, 百, 千 (or their financial forms).
    small_units: [char; 3],
    /// Units for the 10⁴ and 10⁸ groups.
    large_units: [&'static str; 2],
    /// Everyday counting writes 10–19 as 十, 十一, …; the legal (financial)
    /// styles always keep the digit: 壹拾.
    omit_leading_one_before_ten: bool,
}

impl ChineseNumerals {
    const COUNTING: Self = Self {
        digits: ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'],
        small_units: ['十', '百', '千'],
        large_units: ["万", "亿"],
        omit_leading_one_before_ten: true,
    };
    const LEGAL_SIMPLIFIED: Self = Self {
        digits: ['零', '壹', '贰', '叁', '肆', '伍', '陆', '柒', '捌', '玖'],
        small_units: ['拾', '佰', '仟'],
        large_units: ["万", "亿"],
        omit_leading_one_before_ten: false,
    };
    const LEGAL_TRADITIONAL: Self = Self {
        digits: ['零', '壹', '貳', '參', '肆', '伍', '陸', '柒', '捌', '玖'],
        small_units: ['拾', '佰', '仟'],
        large_units: ["萬", "億"],
        omit_leading_one_before_ten: false,
    };
}

/// Chinese place-value counting. Unlike Japanese counting, the digit one is
/// kept before 百 and 千 (一百, 一千), and a run of empty places between
/// non-zero ones is written as a single zero (一百〇一, 一万〇一).
fn to_chinese_counting(value: u32, numerals: ChineseNumerals) -> String {
    let groups = [value / 100_000_000, value / 10_000 % 10_000, value % 10_000];
    let mut result = String::new();
    let mut gap = false;
    for (index, &group) in groups.iter().enumerate() {
        if group == 0 {
            gap |= !result.is_empty();
            continue;
        }
        if !result.is_empty() && (gap || group < 1_000) {
            result.push(numerals.digits[0]);
        }
        gap = false;
        push_chinese_group(&mut result, group, &numerals);
        result.push_str(match index {
            0 => numerals.large_units[1],
            1 => numerals.large_units[0],
            _ => "",
        });
    }
    result
}

fn push_chinese_group(result: &mut String, group: u32, numerals: &ChineseNumerals) {
    let leading = result.is_empty();
    let places = [group / 1_000, group / 100 % 10, group / 10 % 10, group % 10];
    let units = [
        Some(numerals.small_units[2]),
        Some(numerals.small_units[1]),
        Some(numerals.small_units[0]),
        None,
    ];
    let mut started = false;
    let mut zero_run = false;
    for (place, (&digit, unit)) in places.iter().zip(units).enumerate() {
        if digit == 0 {
            zero_run |= started;
            continue;
        }
        if zero_run {
            result.push(numerals.digits[0]);
            zero_run = false;
        }
        let bare_ten =
            numerals.omit_leading_one_before_ten && leading && !started && place == 2 && digit == 1;
        if !bare_ten {
            result.push(numerals.digits[digit as usize]);
        }
        if let Some(unit) = unit {
            result.push(unit);
        }
        started = true;
    }
}

/// Sino-Korean numerals in Hangul: 일, 이, … 십, 십일, … 백, 천, 만. As in
/// Japanese counting the digit one is dropped before a unit, and Korean
/// writes no zero for empty places.
fn to_korean_counting(value: u32) -> String {
    const UNITS: [&str; 4] = ["천", "백", "십", ""];
    const LARGE: [&str; 3] = ["억", "만", ""];
    let groups = [value / 100_000_000, value / 10_000 % 10_000, value % 10_000];
    let mut result = String::new();
    for (index, &group) in groups.iter().enumerate() {
        if group == 0 {
            continue;
        }
        if !(group == 1 && index == 1) {
            let places = [group / 1_000, group / 100 % 10, group / 10 % 10, group % 10];
            for (place, &digit) in places.iter().enumerate() {
                if digit == 0 {
                    continue;
                }
                if digit != 1 || place == 3 {
                    result.push(KOREAN_DIGITS[digit as usize]);
                }
                result.push_str(UNITS[place]);
            }
        }
        result.push_str(LARGE[index]);
    }
    result
}

/// Native Korean numerals (하나, 둘, … 아흔아홉). They stop at 99; larger
/// values are written in decimal.
fn to_korean_native(value: u32) -> String {
    const TENS: [&str; 10] = [
        "", "열", "스물", "서른", "마흔", "쉰", "예순", "일흔", "여든", "아흔",
    ];
    const ONES: [&str; 10] = [
        "", "하나", "둘", "셋", "넷", "다섯", "여섯", "일곱", "여덟", "아홉",
    ];
    if value > 99 {
        return value.to_string();
    }
    format!(
        "{}{}",
        TENS[(value / 10) as usize],
        ONES[(value % 10) as usize]
    )
}

/// Hebrew alphabetic numerals (gematria) for 1–999. 15 and 16 are written
/// טו and טז rather than spelling a divine name. Larger values stay decimal.
fn to_hebrew_numeral(value: u32) -> String {
    const ONES: [&str; 10] = ["", "א", "ב", "ג", "ד", "ה", "ו", "ז", "ח", "ט"];
    const TENS: [&str; 10] = ["", "י", "כ", "ל", "מ", "נ", "ס", "ע", "פ", "צ"];
    const HUNDREDS: [&str; 10] = ["", "ק", "ר", "ש", "ת", "תק", "תר", "תש", "תת", "תתק"];
    if value > 999 {
        return value.to_string();
    }
    let mut result = String::from(HUNDREDS[(value / 100) as usize]);
    match value % 100 {
        15 => result.push_str("טו"),
        16 => result.push_str("טז"),
        rest => {
            result.push_str(TENS[(rest / 10) as usize]);
            result.push_str(ONES[(rest % 10) as usize]);
        }
    }
    result
}

fn multilevel_preset_levels(preset: MultilevelListPreset) -> Vec<NumberingLevel> {
    (0..9)
        .map(|ilvl| {
            let (num_fmt, lvl_text, hanging) = match preset {
                MultilevelListPreset::DecimalHierarchy => (
                    "decimal",
                    format!(
                        "{}.",
                        (0..=ilvl)
                            .map(|index| format!("%{}", index + 1))
                            .collect::<Vec<_>>()
                            .join(".")
                    ),
                    21.6,
                ),
                MultilevelListPreset::ChineseOfficial if ilvl == 0 => {
                    ("chineseCountingThousand", "%1、".to_string(), 21.25)
                }
                MultilevelListPreset::ChineseOfficial if ilvl == 1 => {
                    ("chineseCountingThousand", "(%2)".to_string(), 21.25)
                }
                MultilevelListPreset::ChineseOfficial => {
                    ("decimal", format!("%{}.", ilvl + 1), 18.0)
                }
                MultilevelListPreset::BulletHierarchy => {
                    let glyph = ["•", "○", "■"][ilvl as usize % 3];
                    ("bullet", glyph.to_string(), 18.0)
                }
            };
            let run_props = (num_fmt == "bullet").then(|| RunProperties {
                fonts: Some(RunFonts {
                    ascii: Some(Rc::from("Symbol")),
                    h_ansi: Some(Rc::from("Symbol")),
                    hint: Some(FontHint::Default),
                    ..RunFonts::default()
                }),
                ..RunProperties::default()
            });
            NumberingLevel {
                ilvl,
                start: Some(1),
                restart: None,
                num_fmt: Some(num_fmt.to_string()),
                lvl_text: Some(lvl_text),
                lvl_jc: Some("left".to_string()),
                suffix: Some("tab".to_string()),
                indent: Some(NumberingIndent {
                    left: Some(36.0 * (ilvl + 1) as f32),
                    hanging: Some(hanging),
                }),
                paragraph_style: None,
                paragraph_props: None,
                run_props,
                language: None,
            }
        })
        .collect()
}

fn attr_value(e: &BytesStart, key: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == key {
            return Some(
                attr.unescape_value()
                    .map(|value| value.into_owned())
                    .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).to_string()),
            );
        }
    }

    None
}

fn ensure_run_props(level: &mut NumberingLevel) -> &mut RunProperties {
    level.run_props.get_or_insert_with(RunProperties::default)
}

pub(crate) fn to_roman(num: u32, uppercase: bool) -> String {
    let values = [1000, 900, 500, 400, 100, 90, 50, 40, 10, 9, 5, 4, 1];
    let symbols = if uppercase {
        [
            "M", "CM", "D", "CD", "C", "XC", "L", "XL", "X", "IX", "V", "IV", "I",
        ]
    } else {
        [
            "m", "cm", "d", "cd", "c", "xc", "l", "xl", "x", "ix", "v", "iv", "i",
        ]
    };

    let mut result = String::new();
    let mut n = num;

    for (i, &value) in values.iter().enumerate() {
        while n >= value {
            result.push_str(symbols[i]);
            n -= value;
        }
    }

    result
}

pub(crate) fn to_letter(num: u32, uppercase: bool) -> String {
    const UPPER: [char; 26] = [
        'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J', 'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R',
        'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z',
    ];
    let label = to_repeated_letter(num, &UPPER);
    if uppercase {
        label
    } else {
        label.to_ascii_lowercase()
    }
}

pub(crate) fn to_hex(num: u32) -> String {
    format!("{num:X}")
}

pub(crate) fn is_english_language(language: &str) -> bool {
    language.eq_ignore_ascii_case("en")
        || language
            .get(..3)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("en-"))
}

pub(crate) fn to_english_ordinal(value: u32) -> String {
    // 11, 12, and 13 are the exception even when the final digit would
    // otherwise select st/nd/rd (for example 112th).
    let suffix = if (11..=13).contains(&(value % 100)) {
        "th"
    } else {
        match value % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    format!("{value}{suffix}")
}

pub(crate) fn to_english_cardinal(mut value: u32) -> String {
    if value == 0 {
        return "zero".to_string();
    }
    let mut groups = Vec::new();

    // Word's English CardText output uses short-scale groups and omits the
    // British-style "and" (for example, "seven hundred ninety").
    for (scale, name) in [
        (1_000_000_000, "billion"),
        (1_000_000, "million"),
        (1_000, "thousand"),
    ] {
        let group = value / scale;
        if group > 0 {
            groups.push(format!("{} {name}", to_english_under_thousand(group)));
            value %= scale;
        }
    }

    if value > 0 {
        groups.push(to_english_under_thousand(value));
    }

    groups.join(" ")
}

fn to_english_under_thousand(value: u32) -> String {
    debug_assert!((1..1000).contains(&value));

    const ONES: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 10] = [
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];

    let hundreds = value / 100;
    let remainder = value % 100;
    let mut result = String::new();

    if hundreds > 0 {
        result.push_str(ONES[hundreds as usize]);
        result.push_str(" hundred");
        if remainder > 0 {
            result.push(' ');
        }
    }

    if remainder < 20 {
        if remainder > 0 {
            result.push_str(ONES[remainder as usize]);
        }
    } else {
        result.push_str(TENS[(remainder / 10) as usize]);
        if !remainder.is_multiple_of(10) {
            result.push('-');
            result.push_str(ONES[(remainder % 10) as usize]);
        }
    }

    result
}

pub(crate) fn to_english_ordinal_text(value: u32) -> String {
    let cardinal = to_english_cardinal(value);
    let last_separator = cardinal.rfind([' ', '-']);
    let (prefix, final_word) = match last_separator {
        Some(index) => cardinal.split_at(index + 1),
        None => ("", cardinal.as_str()),
    };

    let ordinal_word = match final_word {
        "one" => "first".to_string(),
        "two" => "second".to_string(),
        "three" => "third".to_string(),
        "five" => "fifth".to_string(),
        "eight" => "eighth".to_string(),
        "nine" => "ninth".to_string(),
        "twelve" => "twelfth".to_string(),
        word if word.ends_with('y') => format!("{}ieth", &word[..word.len() - 1]),
        word => format!("{word}th"),
    };

    format!("{prefix}{ordinal_word}")
}

fn to_full_width_decimal(value: u32) -> String {
    to_decimal_digit_set(value, '０')
}

fn to_decimal_digit_set(value: u32, zero: char) -> String {
    value
        .to_string()
        .chars()
        .map(|ch| {
            // All three digit sets used here are contiguous Unicode blocks.
            // Keeping the zero code point as the parameter makes the mapping
            // shared without hiding which script each numFmt requests.
            char::from_u32(zero as u32 + (ch as u32 - '0' as u32))
                .expect("decimal digit set is valid Unicode")
        })
        .collect()
}

/// Word's half-width `aiueo` alphabet. Small kana and voiced variants are
/// intentionally absent; they are not independent numbering positions.
const AIUEO_KATAKANA_HALF_WIDTH: &str = "ｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜｦﾝ";

const AIUEO_KATAKANA_FULL_WIDTH: &str =
    "アイウエオカキクケコサシスセソタチツテトナニヌネノハヒフヘホマミムメモヤユヨラリルレロワヲン";

/// Unicode has no half-width code points for the historic `ヰ` and `ヱ` used by
/// iroha. Word-compatible output therefore keeps those two glyphs full-width
/// while all representable positions use half-width Katakana.
const IROHA_KATAKANA_HALF_WIDTH: &str = "ｲﾛﾊﾆﾎﾍﾄﾁﾘﾇﾙｦﾜｶﾖﾀﾚｿﾂﾈﾅﾗﾑｳヰﾉｵｸﾔﾏｹﾌｺｴﾃｱｻｷﾕﾒﾐｼヱﾋﾓｾｽ";

const IROHA_KATAKANA_FULL_WIDTH: &str =
    "イロハニホヘトチリヌルヲワカヨタレソツネナラムウヰノオクヤマケフコエテアサキユメミシヱヒモセス";

fn to_kana_sequence(value: u32, alphabet: &str) -> String {
    let alphabet: Vec<char> = alphabet.chars().collect();
    debug_assert!(!alphabet.is_empty());

    // Use the same bijective rollover model as Latin letter numbering: after
    // the last one-character label, continue with the first doubled label.
    // This avoids silently falling back to decimal in long generated lists and
    // keeps every counter value representable without a document-specific cap.
    let radix = alphabet.len() as u32;
    let mut remaining = value.max(1);
    let mut result = Vec::new();
    while remaining > 0 {
        remaining -= 1;
        result.push(alphabet[(remaining % radix) as usize]);
        remaining /= radix;
    }
    result.into_iter().rev().collect()
}

fn to_japanese_positional_digits(value: u32) -> String {
    value
        .max(1)
        .to_string()
        .chars()
        .map(|digit| match digit {
            '0' => '〇',
            '1' => '一',
            '2' => '二',
            '3' => '三',
            '4' => '四',
            '5' => '五',
            '6' => '六',
            '7' => '七',
            '8' => '八',
            '9' => '九',
            _ => unreachable!("u32 decimal rendering contains only ASCII digits"),
        })
        .collect()
}

fn to_enclosed_decimal(value: u32, first_glyph: char, limit: u32) -> String {
    if (1..=limit).contains(&value) {
        // Each supported Unicode block is contiguous in numeric order:
        // ①..⑳, ⒈..⒛, and ⑴..⒇. Keeping the start character explicit makes
        // the mapping auditable against the OOXML examples.
        char::from_u32(first_glyph as u32 + value - 1)
            .expect("enclosed decimal range is valid Unicode")
            .to_string()
    } else {
        value.to_string()
    }
}

fn to_enclosed_ideograph(value: u32) -> String {
    if (1..=10).contains(&value) {
        // Unicode assigns the OOXML example sequence contiguously as
        // CIRCLED IDEOGRAPH ONE (㊀) through TEN (㊉).
        char::from_u32('㊀' as u32 + value - 1)
            .expect("circled ideograph range is valid Unicode")
            .to_string()
    } else {
        // ST_NumberFormat allows non-enclosed equivalents after ten. Preserve
        // the ideographic counting system so the fallback remains script- and
        // width-compatible with the requested format.
        to_japanese_counting(value, JapaneseKanjiStyle::Standard)
    }
}

#[derive(Clone, Copy)]
enum JapaneseKanjiStyle {
    Standard,
    Legal,
}

impl JapaneseKanjiStyle {
    fn digits(self) -> &'static [&'static str; 10] {
        match self {
            Self::Standard => &["", "一", "二", "三", "四", "五", "六", "七", "八", "九"],
            // Word's Japanese legal numbering uses the modern financial forms
            // for the digits that need tamper resistance. Four, six, seven,
            // eight, and nine remain the ordinary common-use characters.
            Self::Legal => &["", "壱", "弐", "参", "四", "伍", "六", "七", "八", "九"],
        }
    }

    fn small_units(self) -> &'static [&'static str; 4] {
        match self {
            Self::Standard => &["千", "百", "十", ""],
            // Microsoft/CLDR examples use 拾 for ten while retaining 百 and 千
            // in this numbering style (as opposed to rarer accounting glyphs
            // such as 佰 and 阡).
            Self::Legal => &["千", "百", "拾", ""],
        }
    }

    fn large_units(self) -> &'static [&'static str; 3] {
        match self {
            Self::Standard => &["", "万", "億"],
            Self::Legal => &["", "萬", "億"],
        }
    }
}

fn to_japanese_counting(value: u32, style: JapaneseKanjiStyle) -> String {
    // Japanese groups large values by 10,000 rather than 1,000. Three groups
    // are sufficient for every u32 value: units, 万, and 億 (up to 42億...).
    let mut remaining = value.max(1);
    let mut groups = [0_u32; 3];
    for group in &mut groups {
        *group = remaining % 10_000;
        remaining /= 10_000;
    }

    let mut result = String::new();
    for group_index in (0..groups.len()).rev() {
        let group = groups[group_index];
        if group == 0 {
            continue;
        }
        result.push_str(&format_japanese_four_digit_group(group, style));
        result.push_str(style.large_units()[group_index]);
    }
    result
}

fn format_japanese_four_digit_group(group: u32, style: JapaneseKanjiStyle) -> String {
    debug_assert!(group < 10_000);
    let divisors = [1_000, 100, 10, 1];
    let digits = style.digits();
    let units = style.small_units();
    let mut result = String::new();

    for (index, divisor) in divisors.into_iter().enumerate() {
        let digit = (group / divisor) % 10;
        if digit == 0 {
            continue;
        }

        // Japanese omits the leading one before 千/百/十: 10 is 十, not 一十.
        // The ones position and large-unit groups keep it, yielding 一万 and
        // 壱萬 rather than a bare unit marker.
        if divisor == 1 || digit != 1 {
            result.push_str(digits[digit as usize]);
        }
        result.push_str(units[index]);
    }
    result
}

/// Keeps track of numbering counters for every numbering instance and level.
/// The structure mimics Word's behaviour where advancing one level resets the
/// deeper levels automatically.
#[derive(Clone, Debug)]
pub struct NumberingCounter {
    counters: HashMap<String, Vec<Option<u32>>>,
    abstract_counters: HashMap<String, Vec<Option<u32>>>,
}

/// The counters that can affect a marker for one concrete numbering instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NumberingCounterState {
    concrete: Vec<Option<u32>>,
    abstract_levels: Vec<Option<u32>>,
}

impl NumberingCounter {
    pub fn state_for(&self, num_id: &str, abstract_num_id: &str) -> NumberingCounterState {
        NumberingCounterState {
            concrete: self.counters.get(num_id).cloned().unwrap_or_default(),
            abstract_levels: self
                .abstract_counters
                .get(abstract_num_id)
                .cloned()
                .unwrap_or_default(),
        }
    }

    /// Compares cache inputs without allocating another snapshot.
    pub fn matches_state(
        &self,
        num_id: &str,
        abstract_num_id: &str,
        state: &NumberingCounterState,
    ) -> bool {
        self.counters.get(num_id).map(Vec::as_slice).unwrap_or(&[]) == state.concrete
            && self
                .abstract_counters
                .get(abstract_num_id)
                .map(Vec::as_slice)
                .unwrap_or(&[])
                == state.abstract_levels
    }

    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
            abstract_counters: HashMap::new(),
        }
    }

    /// A list that starts at a child level still has an implicit parent item.
    /// Seed absent ancestors without advancing ancestors already in use.
    pub fn initialize_ancestor_levels(
        &mut self,
        numbering: Option<&NumberingPart>,
        num_id: &str,
        level: u32,
    ) {
        if level == 0 {
            return;
        }
        if self.counters.get(num_id).is_some_and(|levels| {
            levels.len() >= level as usize && levels[..level as usize].iter().all(Option::is_some)
        }) {
            return;
        }
        let Some(numbering) = numbering else { return };
        let Some(abstract_id) = numbering.abstract_num_id(num_id) else {
            return;
        };
        let concrete = self.counters.entry(num_id.to_owned()).or_default();
        let shared = self
            .abstract_counters
            .entry(abstract_id.to_owned())
            .or_default();
        for ancestor in 0..level {
            if numbering.get_level_info(num_id, ancestor).is_none() {
                continue;
            }
            let index = ancestor as usize;
            concrete.resize(concrete.len().max(index + 1), None);
            shared.resize(shared.len().max(index + 1), None);
            if concrete[index].is_none() {
                let inherited = if numbering.has_start_override(num_id, ancestor) {
                    None
                } else {
                    shared[index]
                };
                concrete[index] = Some(
                    inherited
                        .unwrap_or_else(|| numbering.start_value(num_id, ancestor).unwrap_or(1)),
                );
                shared[index] = concrete[index];
            }
        }
    }

    pub fn increment(
        &mut self,
        num_id: &str,
        abstract_num_id: &str,
        level: u32,
        start: Option<u32>,
        has_start_override: bool,
    ) -> u32 {
        let counters = self.counters.entry(num_id.to_string()).or_default();

        // None distinguishes an unused/reset level from a valid zero start.
        counters.resize(counters.len().max(level as usize + 1), None);
        for value in counters.iter_mut().skip(level as usize + 1) {
            *value = None;
        }

        let counter = &mut counters[level as usize];
        let previous = (*counter).or_else(|| {
            if has_start_override {
                None
            } else {
                self.abstract_counters
                    .get(abstract_num_id)
                    .and_then(|levels| levels.get(level as usize))
                    .copied()
                    .flatten()
            }
        });
        let value = previous.map_or_else(|| start.unwrap_or(1), |previous| previous + 1);
        *counter = Some(value);

        let abstract_levels = self
            .abstract_counters
            .entry(abstract_num_id.to_string())
            .or_default();
        while abstract_levels.len() <= level as usize {
            abstract_levels.push(None);
        }
        for value in abstract_levels.iter_mut().skip((level + 1) as usize) {
            *value = None;
        }
        abstract_levels[level as usize] = *counter;

        value
    }

    pub fn get_counter(&self, num_id: &str, level: u32) -> u32 {
        self.counter_value(num_id, level).unwrap_or(0)
    }

    fn counter_value(&self, num_id: &str, level: u32) -> Option<u32> {
        self.counters
            .get(num_id)
            .and_then(|counters| counters.get(level as usize))
            .copied()
            .flatten()
    }
}

impl Default for NumberingCounter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod decimal_full_width_tests {
    use super::{NumberingCounter, NumberingPart};

    #[test]
    fn formats_decimal_full_width_numbers() {
        let xml = r#"
            <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="301">
                <w:lvl w:ilvl="0">
                  <w:start w:val="12"/>
                  <w:numFmt w:val="decimalFullWidth"/>
                  <w:lvlText w:val="%1"/>
                </w:lvl>
              </w:abstractNum>
              <w:num w:numId="301">
                <w:abstractNumId w:val="301"/>
              </w:num>
            </w:numbering>
        "#;

        let numbering = NumberingPart::from_xml(xml).expect("parse numbering xml");
        let mut counter = NumberingCounter::new();
        let num_id = "301";
        let level = 0;
        counter.increment(
            num_id,
            numbering.abstract_num_id(num_id).unwrap_or_default(),
            level,
            numbering.start_value(num_id, level),
            numbering.has_start_override(num_id, level),
        );

        assert_eq!(numbering.format_number(num_id, level, &counter), "１２");
    }
}

#[cfg(test)]
mod additional_word_number_format_tests {
    use super::{NumberingCounter, NumberingPart};

    fn format_at_start(num_fmt: &str, start: u32) -> String {
        format_at_start_with_template(num_fmt, start, "%1")
    }

    fn format_at_start_with_template(num_fmt: &str, start: u32, template: &str) -> String {
        // Exercise the same parse/counter/template path used by real DOCX
        // numbering. Testing only the eventual kana helper would miss token
        // spelling or level-template integration regressions.
        let xml = format!(
            r#"
            <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="401">
                <w:lvl w:ilvl="0">
                  <w:start w:val="{start}"/>
                  <w:numFmt w:val="{num_fmt}"/>
                  <w:lvlText w:val="{template}"/>
                </w:lvl>
              </w:abstractNum>
              <w:num w:numId="401"><w:abstractNumId w:val="401"/></w:num>
            </w:numbering>
            "#
        );
        let numbering = NumberingPart::from_xml(&xml).expect("parse numbering xml");
        let mut counter = NumberingCounter::new();
        counter.increment("401", "401", 0, numbering.start_value("401", 0), false);
        numbering.format_number("401", 0, &counter)
    }

    #[test]
    fn letters_roll_over_by_repeating_like_word() {
        assert_eq!(format_at_start("upperLetter", 26), "Z");
        assert_eq!(format_at_start("upperLetter", 27), "AA");
        assert_eq!(format_at_start("upperLetter", 28), "BB");
        assert_eq!(format_at_start("lowerLetter", 53), "aaa");
        assert_eq!(format_at_start("russianLower", 29), "аа");
        assert_eq!(format_at_start("russianUpper", 3), "В");
    }

    #[test]
    fn chicago_doubles_its_four_symbols() {
        let labels: Vec<_> = (1..=6).map(|n| format_at_start("chicago", n)).collect();
        assert_eq!(labels, ["*", "†", "‡", "§", "**", "††"]);
    }

    #[test]
    fn stems_branches_and_the_sexagenary_cycle() {
        assert_eq!(format_at_start("ideographTraditional", 1), "甲");
        assert_eq!(format_at_start("ideographTraditional", 10), "癸");
        assert_eq!(format_at_start("ideographTraditional", 11), "甲");
        assert_eq!(format_at_start("ideographZodiac", 12), "亥");
        assert_eq!(format_at_start("ideographZodiacTraditional", 1), "甲子");
        assert_eq!(format_at_start("ideographZodiacTraditional", 11), "甲戌");
        assert_eq!(format_at_start("ideographZodiacTraditional", 60), "癸亥");
        assert_eq!(format_at_start("ideographZodiacTraditional", 61), "甲子");
    }

    #[test]
    fn chinese_counting_keeps_one_before_hundreds_and_marks_gaps_with_zero() {
        for (value, expected) in [
            (1, "一"),
            (10, "十"),
            (11, "十一"),
            (20, "二十"),
            (100, "一百"),
            (101, "一百〇一"),
            (110, "一百一十"),
            (1_001, "一千〇一"),
            (10_000, "一万"),
            (10_001, "一万〇一"),
            (100_000, "十万"),
            (100_000_001, "一亿〇一"),
        ] {
            assert_eq!(
                format_at_start("chineseCounting", value),
                expected,
                "{value}"
            );
        }
        assert_eq!(format_at_start("chineseLegalSimplified", 10), "壹拾");
        assert_eq!(format_at_start("chineseLegalSimplified", 102), "壹佰零贰");
        assert_eq!(format_at_start("ideographLegalTraditional", 30_000), "參萬");
        assert_eq!(format_at_start("taiwaneseDigital", 105), "一〇五");
    }

    #[test]
    fn korean_formats() {
        assert_eq!(format_at_start("ganada", 1), "가");
        assert_eq!(format_at_start("ganada", 14), "하");
        assert_eq!(format_at_start("chosung", 2), "ㄴ");
        assert_eq!(format_at_start("koreanDigital", 10), "일영");
        assert_eq!(format_at_start("koreanCounting", 1), "일");
        assert_eq!(format_at_start("koreanCounting", 11), "십일");
        assert_eq!(format_at_start("koreanCounting", 215), "이백십오");
        assert_eq!(format_at_start("koreanCounting", 10_000), "만");
        assert_eq!(format_at_start("koreanLegal", 1), "하나");
        assert_eq!(format_at_start("koreanLegal", 21), "스물하나");
        assert_eq!(format_at_start("koreanLegal", 100), "100");
    }

    #[test]
    fn hebrew_and_arabic_formats() {
        assert_eq!(format_at_start("hebrew1", 1), "א");
        assert_eq!(format_at_start("hebrew1", 15), "טו");
        assert_eq!(format_at_start("hebrew1", 16), "טז");
        assert_eq!(format_at_start("hebrew1", 115), "קטו");
        assert_eq!(format_at_start("hebrew1", 742), "תשמב");
        assert_eq!(format_at_start("hebrew2", 22), "ת");
        assert_eq!(format_at_start("arabicAlpha", 3), "ت");
        assert_eq!(format_at_start("arabicAbjad", 3), "ج");
    }

    #[test]
    fn shared_formatter_accepts_lowercased_section_tokens() {
        use super::format_number_value;
        assert_eq!(format_number_value("upperroman", 4, None), "IV");
        assert_eq!(format_number_value("decimalfullwidth", 12, None), "１２");
        assert_eq!(format_number_value("japanesecounting", 21, None), "二十一");
        assert_eq!(format_number_value("decimalenclosedcircle", 3, None), "③");
        // Zero has no letter or Roman form; it stays visible as a number.
        assert_eq!(format_number_value("lowerroman", 0, None), "0");
        assert_eq!(format_number_value("upperletter", 0, None), "0");
        assert_eq!(format_number_value("decimalZero", 0, None), "00");
    }

    #[test]
    fn decimal_zero_pads_only_single_digit_values() {
        assert_eq!(format_at_start("decimalZero", 1), "01");
        assert_eq!(format_at_start("decimalZero", 9), "09");
        assert_eq!(format_at_start("decimalZero", 10), "10");
        assert_eq!(format_at_start("decimalZero", 100), "100");
    }

    #[test]
    fn aiueo_matches_word_katakana_width_variants() {
        assert_eq!(format_at_start("aiueo", 1), "ｱ");
        assert_eq!(format_at_start("aiueo", 6), "ｶ");
        assert_eq!(format_at_start("aiueo", 46), "ﾝ");
        assert_eq!(format_at_start("aiueoFullWidth", 1), "ア");
        assert_eq!(format_at_start("aiueoFullWidth", 6), "カ");
        assert_eq!(format_at_start("aiueoFullWidth", 46), "ン");
    }

    #[test]
    fn iroha_matches_word_katakana_width_variants() {
        assert_eq!(format_at_start("iroha", 1), "ｲ");
        assert_eq!(format_at_start("iroha", 2), "ﾛ");
        // Unicode has no half-width variants for the historic iroha kana.
        assert_eq!(format_at_start("iroha", 25), "ヰ");
        assert_eq!(format_at_start("iroha", 43), "ヱ");
        assert_eq!(format_at_start("iroha", 47), "ｽ");
        assert_eq!(format_at_start("irohaFullWidth", 1), "イ");
        assert_eq!(format_at_start("irohaFullWidth", 47), "ス");
    }

    #[test]
    fn japanese_digital_formats_use_positional_kanji_digits() {
        for num_fmt in ["japaneseDigitalTenThousand", "ideographDigital"] {
            assert_eq!(format_at_start(num_fmt, 1), "一");
            assert_eq!(format_at_start(num_fmt, 9), "九");
            assert_eq!(format_at_start(num_fmt, 10), "一〇");
            assert_eq!(format_at_start(num_fmt, 101), "一〇一");
            assert_eq!(format_at_start(num_fmt, 10_000), "一〇〇〇〇");
            assert_eq!(format_at_start(num_fmt, u32::MAX), "四二九四九六七二九五");
        }
    }

    #[test]
    fn japanese_counting_uses_multiplicative_kanji_units() {
        assert_eq!(format_at_start("japaneseCounting", 1), "一");
        assert_eq!(format_at_start("japaneseCounting", 10), "十");
        assert_eq!(format_at_start("japaneseCounting", 11), "十一");
        assert_eq!(format_at_start("japaneseCounting", 105), "百五");
        assert_eq!(format_at_start("japaneseCounting", 10_000), "一万");
        assert_eq!(
            format_at_start("japaneseCounting", 123_456_789),
            "一億二千三百四十五万六千七百八十九"
        );
        assert_eq!(
            format_at_start("japaneseCounting", u32::MAX),
            "四十二億九千四百九十六万七千二百九十五"
        );
    }

    #[test]
    fn japanese_legal_uses_financial_digits_and_units() {
        assert_eq!(format_at_start("japaneseLegal", 1), "壱");
        assert_eq!(format_at_start("japaneseLegal", 2), "弐");
        assert_eq!(format_at_start("japaneseLegal", 3), "参");
        assert_eq!(format_at_start("japaneseLegal", 5), "伍");
        assert_eq!(format_at_start("japaneseLegal", 10), "拾");
        assert_eq!(format_at_start("japaneseLegal", 11), "拾壱");
        assert_eq!(format_at_start("japaneseLegal", 20), "弐拾");
        assert_eq!(format_at_start("japaneseLegal", 10_000), "壱萬");
    }

    #[test]
    fn hex_and_number_in_dash_match_word_examples() {
        assert_eq!(format_at_start("hex", 9), "9");
        assert_eq!(format_at_start("hex", 10), "A");
        assert_eq!(format_at_start("hex", 255), "FF");
        assert_eq!(format_at_start("numberInDash", 12), "- 12 -");
    }

    #[test]
    fn none_suppresses_the_entire_numbering_template() {
        // A formatter that only replaces %1 with an empty string would leak the
        // punctuation from `%1.`. Word's `none` means no numbering label at all.
        assert_eq!(format_at_start_with_template("none", 1, "%1."), "");
    }

    #[test]
    fn enclosed_decimal_formats_use_unicode_number_glyphs_with_documented_limits() {
        assert_eq!(format_at_start("decimalEnclosedCircle", 1), "①");
        assert_eq!(format_at_start("decimalEnclosedCircle", 20), "⑳");
        assert_eq!(format_at_start("decimalEnclosedCircle", 21), "21");

        assert_eq!(format_at_start("decimalEnclosedFullstop", 1), "⒈");
        assert_eq!(format_at_start("decimalEnclosedFullstop", 20), "⒛");
        assert_eq!(format_at_start("decimalEnclosedFullstop", 21), "21");

        assert_eq!(format_at_start("decimalEnclosedParen", 1), "⑴");
        assert_eq!(format_at_start("decimalEnclosedParen", 20), "⒇");
        assert_eq!(format_at_start("decimalEnclosedParen", 21), "21");

        // The Chinese circle variant is specified only through ten.
        assert_eq!(format_at_start("decimalEnclosedCircleChinese", 10), "⑩");
        assert_eq!(format_at_start("decimalEnclosedCircleChinese", 11), "11");
    }

    #[test]
    fn alternate_width_formats_match_their_ooxml_digit_width() {
        // Exercise the public XML/counter/template path so the exact OOXML
        // token spellings remain covered, even though half-width decimal is
        // visually identical to the ordinary decimal formatter.
        assert_eq!(format_at_start("decimalHalfWidth", 12), "12");
        assert_eq!(format_at_start("decimalFullWidth2", 12), "１２");
    }

    #[test]
    fn ideograph_enclosed_circle_uses_circled_kanji_through_ten() {
        assert_eq!(format_at_start("ideographEnclosedCircle", 1), "㊀");
        assert_eq!(format_at_start("ideographEnclosedCircle", 10), "㊉");

        // ST_NumberFormat permits non-enclosed equivalents once the sequence
        // reaches 11. Continue in the matching ideographic counting system
        // instead of falling all the way back to Arabic decimal digits.
        assert_eq!(format_at_start("ideographEnclosedCircle", 11), "十一");
    }

    #[test]
    fn native_decimal_digit_formats_replace_every_place_value() {
        // Use multiple places, including zero, so an implementation cannot
        // accidentally pass by mapping only the first few sequence values.
        assert_eq!(format_at_start("hindiNumbers", 1), "१");
        assert_eq!(format_at_start("hindiNumbers", 2048), "२०४८");
        assert_eq!(format_at_start("thaiNumbers", 1), "๑");
        assert_eq!(format_at_start("thaiNumbers", 2048), "๒๐๔๘");
    }

    fn format_at_start_with_language(num_fmt: &str, start: u32, language: &str) -> String {
        let xml = format!(
            r#"
            <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="402">
                <w:lvl w:ilvl="0">
                  <w:start w:val="{start}"/>
                  <w:numFmt w:val="{num_fmt}"/>
                  <w:lvlText w:val="%1"/>
                  <w:rPr><w:lang w:val="{language}"/></w:rPr>
                </w:lvl>
              </w:abstractNum>
              <w:num w:numId="402"><w:abstractNumId w:val="402"/></w:num>
            </w:numbering>
            "#
        );
        let numbering = NumberingPart::from_xml(&xml).expect("parse localized numbering xml");
        let mut counter = NumberingCounter::new();
        counter.increment("402", "402", 0, numbering.start_value("402", 0), false);
        numbering.format_number("402", 0, &counter)
    }

    #[test]
    fn english_ordinal_uses_language_aware_suffixes() {
        for (value, expected) in [
            (1, "1st"),
            (2, "2nd"),
            (3, "3rd"),
            (4, "4th"),
            (11, "11th"),
            (12, "12th"),
            (13, "13th"),
            (21, "21st"),
            (112, "112th"),
        ] {
            assert_eq!(
                format_at_start_with_language("ordinal", value, "en-US"),
                expected
            );
        }

        // The OOXML format follows the run language. Until another locale is
        // implemented, retain the existing decimal fallback instead of
        // emitting an incorrect English suffix in a Japanese numbering level.
        assert_eq!(format_at_start_with_language("ordinal", 1, "ja-JP"), "1");
    }

    #[test]
    fn english_cardinal_and_ordinal_text_follow_word_wording() {
        for (value, expected) in [
            (1, "one"),
            (21, "twenty-one"),
            (105, "one hundred five"),
            (790, "seven hundred ninety"),
            (1_001, "one thousand one"),
            (
                1_234_567,
                "one million two hundred thirty-four thousand five hundred sixty-seven",
            ),
        ] {
            assert_eq!(
                format_at_start_with_language("cardinalText", value, "en-US"),
                expected
            );
        }

        for (value, expected) in [
            (1, "first"),
            (2, "second"),
            (3, "third"),
            (12, "twelfth"),
            (20, "twentieth"),
            (21, "twenty-first"),
            (105, "one hundred fifth"),
            (790, "seven hundred ninetieth"),
            (1_000, "one thousandth"),
            (1_001, "one thousand first"),
        ] {
            assert_eq!(
                format_at_start_with_language("ordinalText", value, "en-GB"),
                expected
            );
        }

        assert_eq!(
            format_at_start_with_language("cardinalText", 21, "ja-JP"),
            "21"
        );
        assert_eq!(
            format_at_start_with_language("ordinalText", 21, "ja-JP"),
            "21"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{NumberingCounter, NumberingPart};

    #[test]
    fn original_toc_child_first_list_retains_the_implicit_parent_number() {
        let bytes = include_bytes!("../../../public/fixtures/toc.docx");
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("word/numbering.xml").unwrap(),
            &mut xml,
        )
        .unwrap();
        let numbering = NumberingPart::from_xml(&xml).unwrap();
        let mut counter = NumberingCounter::new();
        // Original Word PDF: a,b,c,d,e children precede 2.Test, then
        // a,b children, 3.Test, a child, and 4.tueaeuea.
        for (level, expected) in [
            (1, "a."),
            (1, "b."),
            (1, "c."),
            (1, "d."),
            (1, "e."),
            (0, "2."),
            (1, "a."),
            (1, "b."),
            (0, "3."),
            (1, "a."),
            (0, "4."),
        ] {
            counter.initialize_ancestor_levels(Some(&numbering), "1", level);
            counter.increment(
                "1",
                numbering.abstract_num_id("1").unwrap(),
                level,
                numbering.start_value("1", level),
                numbering.has_start_override("1", level),
            );
            assert_eq!(numbering.format_number("1", level, &counter), expected);
        }
    }

    #[test]
    fn implicit_ancestors_preserve_zero_starts_overrides_and_existing_values() {
        let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:abstractNum w:abstractNumId="0">
            <w:lvl w:ilvl="0"><w:start w:val="0"/></w:lvl>
            <w:lvl w:ilvl="1"><w:start w:val="7"/></w:lvl>
            <w:lvl w:ilvl="2"><w:start w:val="1"/></w:lvl>
          </w:abstractNum>
          <w:num w:numId="a"><w:abstractNumId w:val="0"/></w:num>
          <w:num w:numId="b"><w:abstractNumId w:val="0"/>
            <w:lvlOverride w:ilvl="0"><w:startOverride w:val="9"/></w:lvlOverride>
          </w:num></w:numbering>"#;
        let numbering = NumberingPart::from_xml(xml).unwrap();
        let mut counter = NumberingCounter::new();
        counter.initialize_ancestor_levels(Some(&numbering), "a", 2);
        assert_eq!(counter.counter_value("a", 0), Some(0));
        assert_eq!(counter.counter_value("a", 1), Some(7));
        assert_eq!(counter.increment("a", "0", 0, Some(0), false), 1);
        counter.initialize_ancestor_levels(Some(&numbering), "a", 2);
        assert_eq!(counter.counter_value("a", 0), Some(1));
        assert_eq!(counter.counter_value("a", 1), Some(7));
        let state = counter.state_for("a", "0");
        counter.initialize_ancestor_levels(Some(&numbering), "a", 2);
        assert!(counter.matches_state("a", "0", &state));
        counter.initialize_ancestor_levels(Some(&numbering), "b", 2);
        assert_eq!(counter.counter_value("b", 0), Some(9));
        assert_eq!(counter.counter_value("b", 1), Some(7));
        assert_eq!(counter.increment("b", "0", 0, Some(9), true), 10);
    }

    #[test]
    fn numbering_levels_preserve_character_tracking_in_twips() {
        for (tracking, expected) in [
            ("-20", Some(-1.0)),
            ("10", Some(0.5)),
            ("0", Some(0.0)),
            ("invalid", None),
        ] {
            let xml = format!(
                r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0">
                <w:pPr><w:spacing w:val="60" w:before="120"/></w:pPr>
                <w:rPr><w:spacing w:val="{tracking}"/></w:rPr>
              </w:lvl><w:lvl w:ilvl="1"><w:rPr><w:spacing w:val="-10"/></w:rPr></w:lvl></w:abstractNum>
              <w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num></w:numbering>"#
            );
            let part = NumberingPart::from_xml(&xml).unwrap();
            let level = part.get_level_info("1", 0).unwrap();
            assert_eq!(
                level
                    .run_props
                    .as_ref()
                    .and_then(|props| props.char_spacing),
                expected
            );
            let second_level = part.get_level_info("1", 1).unwrap();
            assert_eq!(
                second_level
                    .run_props
                    .as_ref()
                    .and_then(|props| props.char_spacing),
                Some(-0.5)
            );
        }
    }

    #[test]
    fn zero_start_override_formats_zero_then_one() {
        let xml = r#"<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:abstractNum w:abstractNumId="1"><w:lvl w:ilvl="0">
            <w:start w:val="7"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/>
          </w:lvl></w:abstractNum>
          <w:num w:numId="2"><w:abstractNumId w:val="1"/>
            <w:lvlOverride w:ilvl="0"><w:startOverride w:val="0"/></w:lvlOverride>
          </w:num></w:numbering>"#;
        let numbering = NumberingPart::from_xml(xml).unwrap();
        let mut counters = NumberingCounter::new();
        for expected in ["0.", "1."] {
            counters.increment(
                "2",
                "1",
                0,
                numbering.start_value("2", 0),
                numbering.has_start_override("2", 0),
            );
            assert_eq!(numbering.format_number("2", 0, &counters), expected);
        }
    }

    #[test]
    fn zero_start_advances_and_resets_child_levels() {
        let mut counter = NumberingCounter::new();
        assert_eq!(counter.increment("a", "definition", 0, Some(0), false), 0);
        assert_eq!(counter.increment("a", "definition", 1, Some(0), false), 0);
        assert_eq!(counter.increment("a", "definition", 1, Some(0), false), 1);
        assert_eq!(counter.increment("a", "definition", 0, Some(0), false), 1);
        assert_eq!(counter.increment("a", "definition", 1, Some(0), false), 0);
        assert_eq!(counter.increment("a", "definition", 1, Some(0), false), 1);
    }

    #[test]
    fn zero_start_continues_shared_definition_unless_overridden() {
        let mut counter = NumberingCounter::new();
        assert_eq!(counter.increment("a", "definition", 0, Some(0), false), 0);
        assert_eq!(counter.increment("b", "definition", 0, Some(0), false), 1);
        assert_eq!(counter.increment("c", "definition", 0, Some(0), true), 0);
        assert_eq!(counter.increment("c", "definition", 0, Some(0), true), 1);
    }

    #[test]
    fn unescapes_numbering_text_and_font_attributes() {
        let xml = r#"
            <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="1">
                <w:lvl w:ilvl="0">
                  <w:numFmt w:val="decimal"/>
                  <w:lvlText w:val="%1. &amp; &quot;A&quot; &lt;B&gt;"/>
                  <w:rPr><w:rFonts w:ascii="A&amp;B"/></w:rPr>
                </w:lvl>
              </w:abstractNum>
              <w:num w:numId="1"><w:abstractNumId w:val="1"/></w:num>
            </w:numbering>
        "#;

        let numbering = NumberingPart::from_xml(xml).expect("parse numbering XML");
        let level = numbering.get_level_info("1", 0).expect("numbering level");
        assert_eq!(level.lvl_text.as_deref(), Some("%1. & \"A\" <B>"));
        assert_eq!(
            level
                .run_props
                .as_ref()
                .and_then(|props| props.fonts.as_ref())
                .and_then(|fonts| fonts.ascii.as_deref()),
            Some("A&B")
        );
    }

    #[test]
    fn continues_numbering_across_num_id_without_start_override() {
        let xml = r#"
            <w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:abstractNum w:abstractNumId="16">
                <w:lvl w:ilvl="1">
                  <w:start w:val="41"/>
                  <w:numFmt w:val="decimal"/>
                  <w:lvlText w:val="第%2条"/>
                </w:lvl>
              </w:abstractNum>
              <w:num w:numId="23">
                <w:abstractNumId w:val="16"/>
                <w:lvlOverride w:ilvl="1">
                  <w:startOverride w:val="1"/>
                </w:lvlOverride>
              </w:num>
              <w:num w:numId="21">
                <w:abstractNumId w:val="16"/>
              </w:num>
            </w:numbering>
        "#;

        let numbering = NumberingPart::from_xml(xml).expect("parse numbering xml");
        let mut counter = NumberingCounter::new();

        let first_num_id = "23";
        let first_level = 1;
        counter.increment(
            first_num_id,
            numbering.abstract_num_id(first_num_id).unwrap_or_default(),
            first_level,
            numbering.start_value(first_num_id, first_level),
            numbering.has_start_override(first_num_id, first_level),
        );
        assert_eq!(
            numbering.format_number(first_num_id, first_level, &counter),
            "第1条"
        );

        let second_num_id = "21";
        let second_level = 1;
        counter.increment(
            second_num_id,
            numbering.abstract_num_id(second_num_id).unwrap_or_default(),
            second_level,
            numbering.start_value(second_num_id, second_level),
            numbering.has_start_override(second_num_id, second_level),
        );
        assert_eq!(
            numbering.format_number(second_num_id, second_level, &counter),
            "第2条"
        );
    }
}
