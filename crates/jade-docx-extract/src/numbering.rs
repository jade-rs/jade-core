use jade_number_format::{CUSTOM_FORMAT_PREFIX, format_number_value};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct NumberingDefinitions {
    abstract_nums: HashMap<String, AbstractNum>,
    instances: HashMap<String, NumberingInstance>,
}

#[derive(Debug)]
struct AbstractNum {
    levels: HashMap<u32, NumberingLevel>,
}

#[derive(Debug)]
struct NumberingLevel {
    start: Option<u32>,
    restart: Option<u32>,
    is_legal: bool,
    format: String,
    language: Option<String>,
    text: Option<String>,
    suffix: Option<String>,
}

#[derive(Debug, Default)]
struct NumberingInstance {
    abstract_num_id: String,
    overrides: HashMap<u32, LevelOverride>,
}

#[derive(Debug, Default)]
struct LevelOverride {
    start: Option<u32>,
    level: Option<NumberingLevel>,
}

impl NumberingDefinitions {
    pub fn from_xml(xml: &str) -> Result<Self, String> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut buf = Vec::new();
        let mut definitions = NumberingDefinitions::default();

        let mut current_abstract: Option<PendingAbstract> = None;
        let mut current_level: Option<PendingLevel> = None;
        let mut current_num: Option<PendingInstance> = None;
        let mut current_override_level: Option<u32> = None;

        loop {
            match reader
                .read_event_into(&mut buf)
                .map_err(|e| format!("numbering.xml の解析に失敗しました: {e}"))?
            {
                Event::Start(ref e) => match e.name().as_ref() {
                    b"w:abstractNum" => {
                        if let Some(id) = attr_value(e, b"w:abstractNumId") {
                            current_abstract = Some(PendingAbstract {
                                id,
                                levels: HashMap::new(),
                            });
                        }
                    }
                    b"w:lvl" => {
                        if let Some(ilvl) = attr_value(e, b"w:ilvl")
                            && let Ok(level) = ilvl.parse::<u32>()
                        {
                            current_level = Some(PendingLevel {
                                level,
                                start: None,
                                restart: None,
                                is_legal: false,
                                format: "decimal".to_string(),
                                language: None,
                                text: None,
                                suffix: None,
                            });
                        }
                    }
                    b"w:start" => {
                        if let Some(level) = current_level.as_mut()
                            && let Some(val) = attr_value(e, b"w:val")
                        {
                            level.start = val.parse().ok();
                        }
                    }
                    b"w:lvlRestart" => {
                        if let Some(level) = current_level.as_mut()
                            && let Some(val) = attr_value(e, b"w:val")
                        {
                            level.restart = val.parse().ok();
                        }
                    }
                    b"w:isLgl" => {
                        if let Some(level) = current_level.as_mut() {
                            level.is_legal = attr_value(e, b"w:val").is_none_or(|value| {
                                !matches!(value.as_str(), "0" | "false" | "off")
                            });
                        }
                    }
                    b"w:numFmt" => {
                        if let Some(level) = current_level.as_mut()
                            && let Some(val) = attr_value(e, b"w:val")
                        {
                            if val == "custom" {
                                if let Some(format) = attr_value(e, b"w:format") {
                                    level.format = format!("{CUSTOM_FORMAT_PREFIX}{format}");
                                }
                            } else if !level.format.starts_with(CUSTOM_FORMAT_PREFIX) {
                                // Retain the supported custom choice over its decimal fallback.
                                level.format = val;
                            }
                        }
                    }
                    b"w:lvlText" => {
                        if let Some(level) = current_level.as_mut() {
                            level.text = attr_value(e, b"w:val");
                        }
                    }
                    b"w:lang" => {
                        if let Some(level) = current_level.as_mut() {
                            level.language = attr_value(e, b"w:val");
                        }
                    }
                    b"w:suff" => {
                        if let Some(level) = current_level.as_mut() {
                            level.suffix = attr_value(e, b"w:val");
                        }
                    }
                    b"w:num" => {
                        if let Some(id) = attr_value(e, b"w:numId") {
                            current_num = Some(PendingInstance {
                                id,
                                abstract_num_id: String::new(),
                                overrides: HashMap::new(),
                            });
                        }
                    }
                    b"w:abstractNumId" => {
                        if let Some(num) = current_num.as_mut()
                            && let Some(val) = attr_value(e, b"w:val")
                        {
                            num.abstract_num_id = val;
                        }
                    }
                    b"w:lvlOverride" => {
                        if let Some(ilvl) = attr_value(e, b"w:ilvl")
                            && let Ok(level) = ilvl.parse::<u32>()
                        {
                            current_override_level = Some(level);
                        }
                    }
                    b"w:startOverride" => {
                        if let (Some(level), Some(num)) =
                            (current_override_level, current_num.as_mut())
                            && let Some(val) = attr_value(e, b"w:val")
                        {
                            let overrides = num.overrides.entry(level).or_default();
                            overrides.start = val.parse().ok();
                        }
                    }
                    _ => {}
                },
                Event::Empty(ref e) => {
                    match e.name().as_ref() {
                        b"w:lvl" => {
                            if let Some(ilvl) = attr_value(e, b"w:ilvl")
                                && let Ok(level) = ilvl.parse::<u32>()
                            {
                                current_level = Some(PendingLevel {
                                    level,
                                    start: None,
                                    restart: None,
                                    is_legal: false,
                                    format: "decimal".to_string(),
                                    language: None,
                                    text: None,
                                    suffix: None,
                                });
                            }
                        }
                        b"w:start" => {
                            if let Some(level) = current_level.as_mut()
                                && let Some(val) = attr_value(e, b"w:val")
                            {
                                level.start = val.parse().ok();
                            }
                        }
                        b"w:lvlRestart" => {
                            if let Some(level) = current_level.as_mut()
                                && let Some(val) = attr_value(e, b"w:val")
                            {
                                level.restart = val.parse().ok();
                            }
                        }
                        b"w:isLgl" => {
                            if let Some(level) = current_level.as_mut() {
                                level.is_legal = attr_value(e, b"w:val").is_none_or(|value| {
                                    !matches!(value.as_str(), "0" | "false" | "off")
                                });
                            }
                        }
                        b"w:numFmt" => {
                            if let Some(level) = current_level.as_mut()
                                && let Some(val) = attr_value(e, b"w:val")
                            {
                                if val == "custom" {
                                    if let Some(format) = attr_value(e, b"w:format") {
                                        level.format = format!("{CUSTOM_FORMAT_PREFIX}{format}");
                                    }
                                } else if !level.format.starts_with(CUSTOM_FORMAT_PREFIX) {
                                    // Retain the supported custom choice over its decimal fallback.
                                    level.format = val;
                                }
                            }
                        }
                        b"w:lvlText" => {
                            if let Some(level) = current_level.as_mut() {
                                level.text = attr_value(e, b"w:val");
                            }
                        }
                        b"w:lang" => {
                            if let Some(level) = current_level.as_mut() {
                                level.language = attr_value(e, b"w:val");
                            }
                        }
                        b"w:suff" => {
                            if let Some(level) = current_level.as_mut() {
                                level.suffix = attr_value(e, b"w:val");
                            }
                        }
                        b"w:abstractNumId" => {
                            if let Some(num) = current_num.as_mut()
                                && let Some(val) = attr_value(e, b"w:val")
                            {
                                num.abstract_num_id = val;
                            }
                        }
                        b"w:lvlOverride" => {
                            current_override_level = None;
                        }
                        b"w:startOverride" => {
                            if let (Some(level), Some(num)) =
                                (current_override_level, current_num.as_mut())
                                && let Some(val) = attr_value(e, b"w:val")
                            {
                                let overrides = num.overrides.entry(level).or_default();
                                overrides.start = val.parse().ok();
                            }
                        }
                        _ => {}
                    }
                    handle_end_event(
                        e.name().as_ref(),
                        &mut definitions,
                        &mut current_abstract,
                        &mut current_level,
                        &mut current_num,
                        &mut current_override_level,
                    );
                }
                Event::End(ref e) => {
                    handle_end_event(
                        e.name().as_ref(),
                        &mut definitions,
                        &mut current_abstract,
                        &mut current_level,
                        &mut current_num,
                        &mut current_override_level,
                    );
                }
                Event::Eof => break,
                _ => {}
            }

            buf.clear();
        }

        Ok(definitions)
    }

    pub fn start_value(&self, num_id: &str, level: u32) -> Option<u32> {
        if let Some(instance) = self.instances.get(num_id)
            && let Some(override_level) = instance.overrides.get(&level)
            && override_level.start.is_some()
        {
            return override_level.start;
        }
        self.abstract_level_info(num_id, level)
            .and_then(|info| info.start)
    }

    pub fn abstract_num_id(&self, num_id: &str) -> Option<&str> {
        self.instances
            .get(num_id)
            .map(|instance| instance.abstract_num_id.as_str())
    }

    pub fn has_start_override(&self, num_id: &str, level: u32) -> bool {
        self.instances
            .get(num_id)
            .and_then(|instance| instance.overrides.get(&level))
            .is_some_and(|replacement| replacement.start.is_some())
    }

    pub fn format(&self, num_id: &str, level: u32, counters: &NumberingCounter) -> String {
        let Some(info) = self.level_info(num_id, level) else {
            return counters.get(num_id, level).max(1).to_string();
        };

        if info.format == "none" {
            return String::new();
        }

        let template = info.text.clone().unwrap_or_else(|| "%1.".to_string());

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
                if let Ok(idx) = digits.parse::<u32>()
                    && idx >= 1
                {
                    let placeholder_level = idx - 1;
                    let formatted =
                        self.format_value(num_id, placeholder_level, counters, info.is_legal);
                    result.push_str(&formatted);
                    continue;
                }
                result.push('%');
                result.push_str(&digits);
            } else {
                result.push(ch);
            }
        }

        if let Some(suffix) = info.suffix.as_deref()
            && (suffix.eq_ignore_ascii_case("tab") || suffix.eq_ignore_ascii_case("space"))
        {
            result.push(' ');
        }

        if result.is_empty() {
            let value = counters.get(num_id, level);
            if value > 0 {
                return if info.format == "bullet" {
                    template
                } else {
                    value.to_string()
                };
            }
        }

        result
    }

    fn abstract_level_info(&self, num_id: &str, level: u32) -> Option<&NumberingLevel> {
        let instance = self.instances.get(num_id)?;
        self.abstract_nums
            .get(&instance.abstract_num_id)?
            .levels
            .get(&level)
    }

    fn restart_value(&self, num_id: &str, level: u32) -> Option<u32> {
        // MS-OI29500 2.1.282: Word ignores restart in concrete level overrides.
        self.abstract_level_info(num_id, level)
            .and_then(|info| info.restart)
            .filter(|restart| *restart <= level && *restart <= 7)
    }

    fn level_info(&self, num_id: &str, level: u32) -> Option<&NumberingLevel> {
        let instance = self.instances.get(num_id)?;
        if let Some(replacement) = instance
            .overrides
            .get(&level)
            .and_then(|value| value.level.as_ref())
        {
            return Some(replacement);
        }
        let abstract_num = self.abstract_nums.get(&instance.abstract_num_id)?;
        abstract_num.levels.get(&level)
    }

    fn format_value(
        &self,
        num_id: &str,
        level: u32,
        counters: &NumberingCounter,
        legal: bool,
    ) -> String {
        let value = counters
            .value(num_id, level)
            .unwrap_or_else(|| self.start_value(num_id, level).unwrap_or(1));

        let info = self.level_info(num_id, level);
        let format = info.map(|lvl| lvl.format.as_str()).unwrap_or("decimal");
        // Word applies isLgl to all placeholders, except levels with numFmt=none.
        if legal && format != "none" {
            return value.to_string();
        }
        if format == "bullet" {
            return info
                .and_then(|lvl| lvl.text.clone())
                .unwrap_or_else(|| "•".to_string());
        }
        // Decimal formats can start at zero; other list formats are one-based.
        let value = if matches!(
            format,
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
        format_number_value(format, value, info.and_then(|lvl| lvl.language.as_deref()))
    }
}

struct PendingAbstract {
    id: String,
    levels: HashMap<u32, NumberingLevel>,
}

impl PendingAbstract {
    fn into_pair(self) -> (String, AbstractNum) {
        (
            self.id,
            AbstractNum {
                levels: self.levels,
            },
        )
    }
}

struct PendingLevel {
    level: u32,
    start: Option<u32>,
    restart: Option<u32>,
    is_legal: bool,
    format: String,
    language: Option<String>,
    text: Option<String>,
    suffix: Option<String>,
}

struct PendingInstance {
    id: String,
    abstract_num_id: String,
    overrides: HashMap<u32, LevelOverride>,
}

impl PendingInstance {
    fn into_pair(self) -> (String, NumberingInstance) {
        (
            self.id,
            NumberingInstance {
                abstract_num_id: self.abstract_num_id,
                overrides: self.overrides,
            },
        )
    }
}

fn handle_end_event(
    name: &[u8],
    definitions: &mut NumberingDefinitions,
    current_abstract: &mut Option<PendingAbstract>,
    current_level: &mut Option<PendingLevel>,
    current_num: &mut Option<PendingInstance>,
    current_override_level: &mut Option<u32>,
) {
    match name {
        b"w:lvl" => {
            if let Some(level) = current_level.take() {
                let index = level.level;
                let definition = NumberingLevel {
                    start: level.start,
                    restart: level.restart,
                    is_legal: level.is_legal,
                    format: level.format,
                    language: level.language,
                    text: level.text,
                    suffix: level.suffix,
                };
                if let (Some(index), Some(num)) = (*current_override_level, current_num.as_mut()) {
                    num.overrides.entry(index).or_default().level = Some(definition);
                } else if let Some(abs) = current_abstract.as_mut() {
                    abs.levels.insert(index, definition);
                }
            }
        }
        b"w:abstractNum" => {
            if let Some(pending) = current_abstract.take() {
                let (id, abs) = pending.into_pair();
                definitions.abstract_nums.insert(id, abs);
            }
        }
        b"w:num" => {
            if let Some(pending) = current_num.take() {
                let (id, instance) = pending.into_pair();
                definitions.instances.insert(id, instance);
            }
        }
        b"w:lvlOverride" => {
            *current_override_level = None;
        }
        _ => {}
    }
}

pub struct NumberingCounter {
    counters: HashMap<String, Vec<Option<u32>>>,
    abstract_counters: HashMap<String, Vec<Option<u32>>>,
}

impl NumberingCounter {
    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
            abstract_counters: HashMap::new(),
        }
    }

    pub fn increment(&mut self, numbering: &NumberingDefinitions, num_id: &str, level: u32) -> u32 {
        let abstract_num_id = numbering.abstract_num_id(num_id).unwrap_or_default();
        let restarts = |deeper: usize| {
            numbering
                .restart_value(num_id, deeper as u32)
                .is_none_or(|restart| level < restart)
        };
        let entry = self.counters.entry(num_id.to_string()).or_default();
        entry.resize(entry.len().max(level as usize + 1), None);
        for (deeper, value) in entry.iter_mut().enumerate().skip(level as usize + 1) {
            if restarts(deeper) {
                *value = None;
            }
        }
        let shared = self
            .abstract_counters
            .get(abstract_num_id)
            .and_then(|levels| levels.get(level as usize))
            .copied();
        let previous = match shared {
            Some(value) if !numbering.has_start_override(num_id, level) => value,
            _ => entry[level as usize],
        };
        let value = previous.map_or_else(
            || numbering.start_value(num_id, level).unwrap_or(1),
            |value| value + 1,
        );
        entry[level as usize] = Some(value);
        let shared = self
            .abstract_counters
            .entry(abstract_num_id.to_string())
            .or_default();
        shared.resize(shared.len().max(level as usize + 1), None);
        for (deeper, value) in shared.iter_mut().enumerate().skip(level as usize + 1) {
            if restarts(deeper) {
                *value = None;
            }
        }
        shared[level as usize] = Some(value);
        for (ancestor, previous) in shared.iter().enumerate().take(level as usize) {
            if previous.is_some() && !numbering.has_start_override(num_id, ancestor as u32) {
                entry[ancestor] = *previous;
            }
        }
        value
    }

    fn value(&self, num_id: &str, level: u32) -> Option<u32> {
        self.counters
            .get(num_id)
            .and_then(|levels| levels.get(level as usize))
            .copied()
            .flatten()
    }

    pub fn get(&self, num_id: &str, level: u32) -> u32 {
        self.value(num_id, level).unwrap_or(0)
    }
}

impl Default for NumberingCounter {
    fn default() -> Self {
        Self::new()
    }
}

fn attr_value(e: &BytesStart, key: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == key {
            return Some(
                attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map(|value| value.into_owned())
                    .unwrap_or_else(|_| String::from_utf8_lossy(&attr.value).into_owned()),
            );
        }
    }
    None
}
