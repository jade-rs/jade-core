//! Rich text: a string cell made of runs, each with its own font
//! (`<si><r><rPr>…</rPr><t>…</t></r>…</si>` in the shared strings, or the
//! same inside an inline string's `<is>`).
//!
//! The cell's value stays the joined text; the runs say how to draw it.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::Result;
use crate::model::{CellRef, Color};
use jade_opc::local_name;

/// One run's font. `None` takes the cell's own; a run with an `rPr` states
/// its flags outright, as Excel writes them.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RunFont {
    pub name: Option<String>,
    pub size_points: Option<f32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strike: Option<bool>,
    pub color: Option<Color>,
    /// `superscript` or `subscript`.
    pub vert_align: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TextRun {
    pub text: String,
    pub font: RunFont,
}

/// The runs of each shared string, `None` for a plain one.
pub fn parse_shared_runs(xml: &str) -> Result<Vec<Option<Vec<TextRun>>>> {
    let content = crate::cell_content::shared_strings(xml)?;
    let mut reader = Reader::from_str(&content);
    reader.config_mut().trim_text(false);
    let mut pool = Vec::new();
    let mut runs: Option<Vec<TextRun>> = None;
    let mut parser = RunParser::default();
    let mut buf = Vec::new();
    loop {
        let event = reader.read_event_into(&mut buf)?;
        match &event {
            Event::Start(element) if local_name(element.name().as_ref()) == b"si" => {
                runs = Some(Vec::new());
            }
            Event::End(element) if local_name(element.name().as_ref()) == b"si" => {
                let found = runs.take().unwrap_or_default();
                pool.push((!found.is_empty()).then_some(found));
            }
            Event::Eof => break,
            _ => {
                if let Some(runs) = runs.as_mut() {
                    parser.feed(&event, runs)?;
                }
            }
        }
        buf.clear();
    }
    Ok(pool)
}

/// The rich text cells of a worksheet: inline strings with runs, and shared
/// strings whose entry in `shared` has runs.
pub fn sheet_runs(
    xml: &str,
    shared: &[Option<Vec<TextRun>>],
) -> Result<BTreeMap<CellRef, Vec<TextRun>>> {
    let mut found = BTreeMap::new();
    if !xml.contains("inlineStr") && shared.iter().all(Option::is_none) {
        return Ok(found);
    }
    let content = crate::cell_content::worksheet(xml)?;
    let mut reader = Reader::from_str(&content);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    // The cell being read: where, and how its value is stored.
    let mut cell: Option<(CellRef, String)> = None;
    let mut in_value = false;
    let mut value = String::new();
    let mut inline: Option<Vec<TextRun>> = None;
    let mut parser = RunParser::default();
    loop {
        let event = reader.read_event_into(&mut buf)?;
        match &event {
            Event::Start(element) | Event::Empty(element)
                if local_name(element.name().as_ref()) == b"c" =>
            {
                let reference =
                    attribute(element, b"r")?.and_then(|text| CellRef::parse(&text).ok());
                let kind = attribute(element, b"t")?.unwrap_or_default();
                cell = reference
                    .filter(|_| matches!(event, Event::Start(_)))
                    .map(|reference| (reference, kind));
                value.clear();
            }
            Event::Start(element) if cell.is_some() => match local_name(element.name().as_ref()) {
                b"v" => in_value = true,
                b"is" => inline = Some(Vec::new()),
                _ => {
                    if let Some(runs) = inline.as_mut() {
                        parser.feed(&event, runs)?;
                    }
                }
            },
            Event::Text(text) if in_value => value.push_str(&text.decode()?),
            Event::CData(text) if in_value => value.push_str(&text.decode()?),
            Event::GeneralRef(reference) if in_value => {
                if let Some(character) = reference.resolve_char_ref()? {
                    value.push(character);
                }
            }
            Event::End(element) => match local_name(element.name().as_ref()) {
                b"v" => in_value = false,
                b"is" => {
                    if let (Some((reference, _)), Some(runs)) = (&cell, inline.take()) {
                        if !runs.is_empty() {
                            found.insert(*reference, runs);
                        }
                    }
                }
                b"c" => {
                    if let Some((reference, kind)) = cell.take() {
                        if kind == "s" {
                            if let Some(Some(runs)) = value
                                .trim()
                                .parse::<usize>()
                                .ok()
                                .and_then(|index| shared.get(index))
                            {
                                found.insert(reference, runs.clone());
                            }
                        }
                    }
                }
                _ => {
                    if let Some(runs) = inline.as_mut() {
                        parser.feed(&event, runs)?;
                    }
                }
            },
            Event::Eof => break,
            _ => {
                if let Some(runs) = inline.as_mut() {
                    parser.feed(&event, runs)?;
                }
            }
        }
        buf.clear();
    }
    Ok(found)
}

/// Reads `<r>` runs from the events inside an `<si>` or `<is>`.
#[derive(Default)]
struct RunParser {
    run: Option<TextRun>,
    in_properties: bool,
    in_text: bool,
    /// Inside `<rPh>`: furigana, not part of the value.
    phonetic: usize,
}

impl RunParser {
    fn feed(&mut self, event: &Event<'_>, runs: &mut Vec<TextRun>) -> Result<()> {
        match event {
            Event::Start(element) | Event::Empty(element) => {
                let empty = matches!(event, Event::Empty(_));
                let qualified = element.name();
                let name = local_name(qualified.as_ref());
                match name {
                    b"rPh" if !empty => self.phonetic += 1,
                    _ if self.phonetic > 0 => {}
                    b"r" if !empty => self.run = Some(TextRun::default()),
                    b"rPr" => {
                        if let Some(run) = self.run.as_mut() {
                            // A run that states its font states every flag.
                            run.font.bold = Some(false);
                            run.font.italic = Some(false);
                            run.font.underline = Some(false);
                            run.font.strike = Some(false);
                            self.in_properties = !empty;
                        }
                    }
                    b"t" if !empty && self.run.is_some() => self.in_text = true,
                    _ if self.in_properties => {
                        if let Some(run) = self.run.as_mut() {
                            let flag = || -> Result<bool> {
                                Ok(attribute(element, b"val")?
                                    .is_none_or(|value| !matches!(value.as_str(), "0" | "false")))
                            };
                            match name {
                                b"b" => run.font.bold = Some(flag()?),
                                b"i" => run.font.italic = Some(flag()?),
                                b"strike" => run.font.strike = Some(flag()?),
                                b"u" => {
                                    run.font.underline = Some(
                                        attribute(element, b"val")?
                                            .is_none_or(|value| value != "none"),
                                    )
                                }
                                b"sz" => {
                                    run.font.size_points = attribute(element, b"val")?
                                        .and_then(|value| value.parse().ok())
                                }
                                b"rFont" | b"name" => run.font.name = attribute(element, b"val")?,
                                b"color" => run.font.color = crate::styles::parse_color(element)?,
                                b"vertAlign" => {
                                    run.font.vert_align = attribute(element, b"val")?
                                        .filter(|value| value != "baseline")
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(text) if self.in_text => {
                if let Some(run) = self.run.as_mut() {
                    run.text.push_str(&text.decode()?);
                }
            }
            Event::CData(text) if self.in_text => {
                if let Some(run) = self.run.as_mut() {
                    run.text.push_str(&text.decode()?);
                }
            }
            Event::GeneralRef(reference) if self.in_text => {
                if let Some(run) = self.run.as_mut() {
                    if let Some(character) = reference.resolve_char_ref()? {
                        run.text.push(character);
                    } else if let Some(text) =
                        quick_xml::escape::resolve_predefined_entity(&reference.decode()?)
                    {
                        run.text.push_str(text);
                    }
                }
            }
            Event::End(element) => match local_name(element.name().as_ref()) {
                b"rPh" => self.phonetic = self.phonetic.saturating_sub(1),
                b"rPr" => self.in_properties = false,
                b"t" => self.in_text = false,
                b"r" if self.phonetic == 0 => {
                    if let Some(mut run) = self.run.take() {
                        run.text = crate::xstring::decode_xstring(&run.text);
                        runs.push(run);
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }
}

fn attribute(element: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for attribute in element.attributes() {
        let attribute = attribute?;
        if local_name(attribute.key.as_ref()) == key {
            return Ok(Some(attribute.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_strings_keep_their_runs_and_plain_ones_have_none() {
        let pool = parse_shared_runs(
            r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>plain</t></si><si><r><rPr><b/><sz val="14"/><color rgb="FFFF0000"/><rFont val="Arial"/></rPr><t>Bold</t></r><r><t xml:space="preserve"> and </t></r><r><rPr><vertAlign val="superscript"/></rPr><t>2</t></r><rPh sb="0" eb="1"><t>ヨミ</t></rPh></si></sst>"#,
        )
        .unwrap();
        assert_eq!(pool.len(), 2);
        assert!(pool[0].is_none());
        let runs = pool[1].as_ref().unwrap();
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].text, "Bold");
        assert_eq!(runs[0].font.bold, Some(true));
        assert_eq!(runs[0].font.size_points, Some(14.0));
        assert_eq!(runs[0].font.name.as_deref(), Some("Arial"));
        assert_eq!(runs[0].font.color, Some(Color::Argb([255, 255, 0, 0])));
        assert_eq!(runs[1].text, " and ");
        assert_eq!(runs[1].font, RunFont::default());
        assert_eq!(runs[2].font.vert_align.as_deref(), Some("superscript"));
        assert_eq!(runs[2].font.bold, Some(false));
    }

    #[test]
    fn worksheets_find_their_rich_cells() {
        let shared = vec![None, Some(vec![TextRun::default()])];
        let found = sheet_runs(
            r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1" t="inlineStr"><is><r><rPr><i/></rPr><t>x</t></r></is></c><c r="D1" t="inlineStr"><is><t>plain</t></is></c></row></sheetData></worksheet>"#,
            &shared,
        )
        .unwrap();
        assert_eq!(found.len(), 2);
        assert!(found.contains_key(&CellRef::parse("B1").unwrap()));
        let inline = &found[&CellRef::parse("C1").unwrap()];
        assert_eq!(inline[0].text, "x");
        assert_eq!(inline[0].font.italic, Some(true));
    }
}
