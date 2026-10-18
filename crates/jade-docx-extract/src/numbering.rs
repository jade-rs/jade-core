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
    format: NumberFormat,
    text: Option<String>,
    suffix: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum NumberFormat {
    Decimal,
    DecimalZero,
    UpperRoman,
    LowerRoman,
    UpperLetter,
    LowerLetter,
    Bullet,
    Other,
}

#[derive(Debug, Default)]
struct NumberingInstance {
    abstract_num_id: String,
    overrides: HashMap<u32, LevelOverride>,
}

#[derive(Debug, Default)]
struct LevelOverride {
    start: Option<u32>,
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
                                format: NumberFormat::Decimal,
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
                    b"w:numFmt" => {
                        if let Some(level) = current_level.as_mut()
                            && let Some(val) = attr_value(e, b"w:val")
                        {
                            level.format = NumberFormat::from_str(&val);
                        }
                    }
                    b"w:lvlText" => {
                        if let Some(level) = current_level.as_mut() {
                            level.text = attr_value(e, b"w:val");
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
                        b"w:start" => {
                            if let Some(level) = current_level.as_mut()
                                && let Some(val) = attr_value(e, b"w:val")
                            {
                                level.start = val.parse().ok();
                            }
                        }
                        b"w:numFmt" => {
                            if let Some(level) = current_level.as_mut()
                                && let Some(val) = attr_value(e, b"w:val")
                            {
                                level.format = NumberFormat::from_str(&val);
                            }
                        }
                        b"w:lvlText" => {
                            if let Some(level) = current_level.as_mut() {
                                level.text = attr_value(e, b"w:val");
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
        if let Some(instance) = self.instances.get(num_id) {
            if let Some(override_level) = instance.overrides.get(&level)
                && override_level.start.is_some()
            {
                return override_level.start;
            }
            if let Some(abs) = self.abstract_nums.get(&instance.abstract_num_id)
                && let Some(level_info) = abs.levels.get(&level)
            {
                return level_info.start;
            }
        }

        None
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
            .and_then(|override_level| override_level.start)
            .is_some()
    }

    pub fn format(&self, num_id: &str, level: u32, counters: &NumberingCounter) -> String {
        let Some(info) = self.level_info(num_id, level) else {
            return counters.get(num_id, level).max(1).to_string();
        };

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
                    let formatted = self.format_value(num_id, placeholder_level, counters);
                    if !formatted.is_empty() {
                        result.push_str(&formatted);
                        continue;
                    }
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
                return match info.format {
                    NumberFormat::Bullet => template,
                    _ => value.to_string(),
                };
            }
        }

        result
    }

    fn level_info(&self, num_id: &str, level: u32) -> Option<&NumberingLevel> {
        let instance = self.instances.get(num_id)?;
        let abstract_num = self.abstract_nums.get(&instance.abstract_num_id)?;
        abstract_num.levels.get(&level)
    }

    fn format_value(&self, num_id: &str, level: u32, counters: &NumberingCounter) -> String {
        let value = counters.get(num_id, level);
        let value = if value == 0 {
            self.start_value(num_id, level).unwrap_or(1)
        } else {
            value
        };

        let format = self
            .level_info(num_id, level)
            .map(|lvl| lvl.format)
            .unwrap_or(NumberFormat::Decimal);

        match format {
            NumberFormat::Decimal => value.to_string(),
            NumberFormat::DecimalZero => format!("{value:02}"),
            NumberFormat::UpperRoman => to_roman(value, true),
            NumberFormat::LowerRoman => to_roman(value, false),
            NumberFormat::UpperLetter => to_letter(value, true),
            NumberFormat::LowerLetter => to_letter(value, false),
            NumberFormat::Bullet => self
                .level_info(num_id, level)
                .and_then(|lvl| lvl.text.clone())
                .unwrap_or_else(|| "•".to_string()),
            NumberFormat::Other => value.to_string(),
        }
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
    format: NumberFormat,
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
            if let Some(level) = current_level.take()
                && let Some(abs) = current_abstract.as_mut()
            {
                abs.levels.insert(
                    level.level,
                    NumberingLevel {
                        start: level.start,
                        format: level.format,
                        text: level.text,
                        suffix: level.suffix,
                    },
                );
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
    counters: HashMap<String, Vec<u32>>,
    abstract_counters: HashMap<String, Vec<u32>>,
}

impl NumberingCounter {
    pub fn new() -> Self {
        Self {
            counters: HashMap::new(),
            abstract_counters: HashMap::new(),
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
        let entry = self.counters.entry(num_id.to_string()).or_default();

        while entry.len() <= level as usize {
            entry.push(0);
        }

        for value in entry.iter_mut().skip((level + 1) as usize) {
            *value = 0;
        }

        let counter = &mut entry[level as usize];
        if *counter == 0 {
            // Keep numbering continuous across numId changes when both ids point
            // to the same abstractNum and the new numId has no startOverride.
            if !has_start_override {
                if let Some(previous) = self
                    .abstract_counters
                    .get(abstract_num_id)
                    .and_then(|levels| levels.get(level as usize))
                    .copied()
                    .filter(|value| *value > 0)
                {
                    *counter = previous + 1;
                } else {
                    *counter = start.unwrap_or(1);
                }
            } else {
                *counter = start.unwrap_or(1);
            }
        } else {
            *counter += 1;
        }

        let abstract_entry = self
            .abstract_counters
            .entry(abstract_num_id.to_string())
            .or_default();
        while abstract_entry.len() <= level as usize {
            abstract_entry.push(0);
        }
        for value in abstract_entry.iter_mut().skip((level + 1) as usize) {
            *value = 0;
        }
        abstract_entry[level as usize] = *counter;

        *counter
    }

    pub fn get(&self, num_id: &str, level: u32) -> u32 {
        self.counters
            .get(num_id)
            .and_then(|levels| levels.get(level as usize).copied())
            .unwrap_or(0)
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
            return Some(String::from_utf8_lossy(&attr.value).to_string());
        }
    }
    None
}

impl NumberFormat {
    fn from_str(value: &str) -> Self {
        match value {
            "decimal" => NumberFormat::Decimal,
            "decimalZero" => NumberFormat::DecimalZero,
            "upperRoman" => NumberFormat::UpperRoman,
            "lowerRoman" => NumberFormat::LowerRoman,
            "upperLetter" => NumberFormat::UpperLetter,
            "lowerLetter" => NumberFormat::LowerLetter,
            "bullet" => NumberFormat::Bullet,
            _ => NumberFormat::Other,
        }
    }
}

fn to_roman(mut num: u32, uppercase: bool) -> String {
    if num == 0 {
        num = 1;
    }

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

/// Word's lettered lists repeat the letter after `z`: `aa`, `bb`, `cc`, …
fn to_letter(num: u32, uppercase: bool) -> String {
    let index = num.max(1) - 1;
    if index / 26 >= 40 {
        // A hostile w:start must not turn into a megabyte-long label.
        return num.to_string();
    }
    let base = if uppercase { b'A' } else { b'a' };
    let letter = (base + (index % 26) as u8) as char;
    std::iter::repeat_n(letter, (index / 26 + 1) as usize).collect()
}
