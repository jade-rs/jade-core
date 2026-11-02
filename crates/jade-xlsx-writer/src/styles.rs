//! Adding a cell format to `styles.xml`.
//!
//! A cell wears one `cellXfs` entry, which points at a font, a fill, a
//! border and a number format. Making a cell bold therefore means a new
//! font (the old one with `<b/>`), a new `xf` pointing at it, and the cell
//! pointed at that `xf` — never editing the entries it shares with other
//! cells. Like the sheet patcher, this splices text into the part so
//! everything it does not touch — theme colours, `dxfs`, table styles,
//! extension lists — comes back exactly as it was.
//!
//! Entries are reused when an identical one exists, so pressing Ctrl+B ten
//! times adds one font and one `xf`, not ten.

use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};

pub use jade_xlsx_headless_api::{BorderLine, BorderStyle};
use jade_xlsx_reader::{builtin_number_format, HorizontalAlignment, VerticalAlignment};

use crate::patch::{
    apply, attribute, element_head_with_attribute, escape, local, namespace_prefix, Splice,
};
use crate::{Result, WriteError};

pub(crate) fn minimal_styles(strict: bool) -> String {
    let namespace = if strict {
        "http://purl.oclc.org/ooxml/spreadsheetml/main"
    } else {
        "http://schemas.openxmlformats.org/spreadsheetml/2006/main"
    };
    format!("<styleSheet xmlns=\"{namespace}\"><fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/></cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles></styleSheet>")
}

/// Appends a namespaced child without assuming the parent's prefix or whether
/// the root uses an empty tag. Existing relationships/types remain untouched.
fn append_root(xml: &str, child: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Empty(element) if depth == 0 => {
                let name = String::from_utf8_lossy(element.name().as_ref()).into_owned();
                let end = reader.buffer_position() as usize;
                let head = xml[start..end].trim_end().trim_end_matches("/>");
                return Ok(apply(
                    xml,
                    vec![Splice {
                        start,
                        end,
                        text: format!("{head}>{child}</{name}>"),
                    }],
                ));
            }
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return Ok(apply(
                        xml,
                        vec![Splice {
                            start,
                            end: start,
                            text: child.into(),
                        }],
                    ));
                }
            }
            Event::Eof => return Err(WriteError::NoStyleTables),
            _ => {}
        }
    }
}

pub(crate) fn install_styles_relationship(
    package: &mut jade_opc::OpcPackage,
    part: &str,
) -> Result<()> {
    let workbook = crate::workbook_part_name(package)?;
    let rels_name = jade_opc::rels_part_name(&workbook);
    let rels = package.part_str(&rels_name)?.unwrap_or_else(|| {
        "<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>".into()
    });
    let relationships = jade_opc::parse_relationships(&rels)?;
    if relationships.target_by_type_suffix("/styles").is_none() {
        let mut id = 1;
        while relationships
            .relationship(&format!("jadeStyles{id}"))
            .is_some()
        {
            id += 1;
        }
        let strict = package
            .part_str(&workbook)?
            .is_some_and(|xml| xml.contains("http://purl.oclc.org/ooxml/spreadsheetml/main"));
        let relationship_namespace = if strict {
            "http://purl.oclc.org/ooxml/officeDocument/relationships"
        } else {
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
        };
        let entry = format!("<Relationship xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\" Id=\"jadeStyles{id}\" Type=\"{relationship_namespace}/styles\" Target=\"/{}\"/>", escape(part));
        package.replace(&rels_name, append_root(&rels, &entry)?.into_bytes());
    }
    let types = package
        .part_str("[Content_Types].xml")?
        .ok_or_else(|| WriteError::MissingPart("[Content_Types].xml".into()))?;
    let mut reader = Reader::from_str(&types);
    let mut found = false;
    loop {
        match reader.read_event()? {
            Event::Start(element) | Event::Empty(element)
                if local(element.name().as_ref()) == b"Override" =>
            {
                if attribute(&element, b"PartName")?.as_deref() == Some(&format!("/{part}")) {
                    found = true;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !found {
        let entry = format!("<Override xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\" PartName=\"/{}\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>", escape(part));
        package.replace(
            "[Content_Types].xml",
            append_root(&types, &entry)?.into_bytes(),
        );
    }
    Ok(())
}

/// What to change about a cell's format. Every field is optional; `None`
/// keeps what the cell has.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FormatChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,
    /// Whether content edits require an unprotected sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    /// Suppress contents in the formula bar while the sheet is protected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula_hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strike: Option<bool>,
    /// `RRGGBB`, or a theme colour kept as one: `theme:N` (0–11, the
    /// theme's `dk1`, `lt1`, `dk2`, `lt2`, `accent1`–`6`, `hlink`,
    /// `folHlink`) with an optional tint, `theme:4:0.4` for 40% lighter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_name: Option<String>,
    /// A solid fill's colour, as [`FormatChange::font_color`] takes it;
    /// the empty string for no fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<String>,
    /// A format code such as `0.00` or `yyyy/mm/dd`; `General` or the
    /// empty string for General.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizontal: Option<HorizontalAlignment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical: Option<VerticalAlignment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_text: Option<bool>,
    /// Shrink the text to fit the column instead of overflowing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shrink_to_fit: Option<bool>,
    /// Indent level (0–250), each three spaces of the Normal font. Excel
    /// keeps it only with left, right or distributed alignment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indent: Option<u32>,
    /// SpreadsheetML's rotation: 0–90 degrees up, 91–180 for 1–90 down,
    /// 255 for vertical (stacked) text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_rotation: Option<u16>,
    /// The left edge; a line of style `none` removes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_left: Option<BorderLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_right: Option<BorderLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_top: Option<BorderLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_bottom: Option<BorderLine>,
    /// The line both diagonals share. It shows only in the directions
    /// `diagonal_up` / `diagonal_down` turn on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border_diagonal: Option<BorderLine>,
    /// Bottom-left to top-right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagonal_up: Option<bool>,
    /// Top-left to bottom-right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagonal_down: Option<bool>,
}

impl FormatChange {
    /// Whether anything is asked for at all.
    pub fn is_empty(&self) -> bool {
        *self == FormatChange::default()
    }

    fn touches_font(&self) -> bool {
        self.bold.is_some()
            || self.italic.is_some()
            || self.underline.is_some()
            || self.strike.is_some()
            || self.font_color.is_some()
            || self.font_size.is_some()
            || self.font_name.is_some()
    }

    fn touches_border(&self) -> bool {
        self.border_left.is_some()
            || self.border_right.is_some()
            || self.border_top.is_some()
            || self.border_bottom.is_some()
            || self.border_diagonal.is_some()
            || self.diagonal_up.is_some()
            || self.diagonal_down.is_some()
    }

    fn touches_alignment(&self) -> bool {
        self.horizontal.is_some()
            || self.vertical.is_some()
            || self.wrap_text.is_some()
            || self.shrink_to_fit.is_some()
            || self.indent.is_some()
            || self.text_rotation.is_some()
    }
}

/// An element found in the part: where it is, and its text.
#[derive(Debug, Clone)]
struct Found {
    start: usize,
    end: usize,
    /// Whether it was written `<x/>`.
    empty: bool,
}

/// A `<fonts>`-style list: its opening tag, its entries and where `</…>`
/// begins.
#[derive(Debug, Clone, Default)]
struct List {
    open: Option<Found>,
    entries: Vec<Found>,
    close_at: Option<usize>,
}

#[derive(Debug, Default)]
struct Layout {
    num_fmts: List,
    fonts: List,
    fills: List,
    borders: List,
    cell_xfs: List,
    /// Where `<fonts>` starts, for a `<numFmts>` that has to be created.
    fonts_at: Option<usize>,
    /// Where the fill list ends, for a `<borders>` that has to be created.
    fills_end: Option<usize>,
    prefix: String,
}

/// Adds the format `change` makes of `base` (a `cellXfs` index; `None`
/// is the default style) to `styles_xml` and returns the new part text and
/// the index of the `xf` to give cells. When such an `xf` already exists the
/// text comes back as it was, with that index.
pub fn add_cell_format(
    styles_xml: &str,
    base: Option<u32>,
    change: &FormatChange,
) -> Result<(String, u32)> {
    if change
        .font_size
        .is_some_and(|size| !size.is_finite() || !(1.0..=409.0).contains(&size))
    {
        return Err(WriteError::BadFontSize);
    }
    if change.indent.is_some_and(|indent| indent > 250) {
        return Err(WriteError::BadAlignment("an indent is 0 to 250"));
    }
    if change
        .text_rotation
        .is_some_and(|rotation| rotation > 180 && rotation != 255)
    {
        return Err(WriteError::BadAlignment(
            "a rotation is 0 to 180, or 255 for vertical text",
        ));
    }
    let layout = scan(styles_xml)?;
    let base_index = base.unwrap_or(0) as usize;
    let base_xf = layout
        .cell_xfs
        .entries
        .get(base_index)
        .ok_or(WriteError::NoSuchStyle {
            index: base_index as u32,
        })?;
    let base_xf_text = &styles_xml[base_xf.start..base_xf.end];
    let mut xf_attributes = attributes_of(base_xf_text)?;
    let mut alignment = child_attributes(base_xf_text, "alignment")?;
    let mut other_children = children_except(base_xf_text, &["alignment"])?;
    other_children.retain(|child| !child.is_empty());

    let mut splices: Vec<Splice> = Vec::new();

    // ── font ────────────────────────────────────────────────────────
    if change.touches_font() {
        let font_index = attribute_number(&xf_attributes, "fontId").unwrap_or(0) as usize;
        let base_font = layout
            .fonts
            .entries
            .get(font_index)
            .ok_or(WriteError::NoSuchStyle {
                index: font_index as u32,
            })?;
        let font_text = derive_font(
            &styles_xml[base_font.start..base_font.end],
            change,
            &layout.prefix,
        )?;
        let index = match find_entry(styles_xml, &layout.fonts, &font_text) {
            Some(index) => index,
            None => append_entry(styles_xml, &layout.fonts, &font_text, &mut splices)?,
        };
        set_attribute(&mut xf_attributes, "fontId", &index.to_string());
        set_attribute(&mut xf_attributes, "applyFont", "1");
    }

    // ── fill ────────────────────────────────────────────────────────
    if let Some(fill) = &change.fill_color {
        let index = if fill.is_empty() {
            0
        } else {
            let color = color_attributes(fill)?;
            let fill_text = format!(
                "<{p}fill><{p}patternFill patternType=\"solid\"><{p}fgColor {color}/><{p}bgColor indexed=\"64\"/></{p}patternFill></{p}fill>",
                p = layout.prefix
            );
            match find_entry(styles_xml, &layout.fills, &fill_text) {
                Some(index) => index,
                None => append_entry(styles_xml, &layout.fills, &fill_text, &mut splices)?,
            }
        };
        set_attribute(&mut xf_attributes, "fillId", &index.to_string());
        set_attribute(&mut xf_attributes, "applyFill", "1");
    }

    // ── border ──────────────────────────────────────────────────────
    if change.touches_border() {
        let border_index = attribute_number(&xf_attributes, "borderId").unwrap_or(0) as usize;
        let base_border = layout
            .borders
            .entries
            .get(border_index)
            .map(|found| &styles_xml[found.start..found.end]);
        let border_text = derive_border(base_border, change, &layout.prefix)?;
        let index = if layout.borders.open.is_some() {
            match find_entry(styles_xml, &layout.borders, &border_text) {
                Some(index) => index,
                None => append_entry(styles_xml, &layout.borders, &border_text, &mut splices)?,
            }
        } else {
            // No border list at all: create one in schema order, right
            // after the fills, with the "no border" every other xf means.
            let at = layout.fills_end.ok_or(WriteError::NoStyleTables)?;
            let none = derive_border(None, &FormatChange::default(), &layout.prefix)?;
            let (entries, count, index) = if squeeze(&border_text) == squeeze(&none) {
                (none, 1, 0)
            } else {
                (format!("{none}{border_text}"), 2, 1)
            };
            splices.push(Splice {
                start: at,
                end: at,
                text: format!(
                    "<{p}borders count=\"{count}\">{entries}</{p}borders>",
                    p = layout.prefix
                ),
            });
            index
        };
        set_attribute(&mut xf_attributes, "borderId", &index.to_string());
        set_attribute(&mut xf_attributes, "applyBorder", "1");
    }

    // ── number format ───────────────────────────────────────────────
    if let Some(code) = &change.number_format {
        let id = number_format_id(styles_xml, &layout, code, &mut splices)?;
        set_attribute(&mut xf_attributes, "numFmtId", &id.to_string());
        set_attribute(&mut xf_attributes, "applyNumberFormat", "1");
    }

    // ── alignment ───────────────────────────────────────────────────
    if change.touches_alignment() {
        if let Some(horizontal) = change.horizontal {
            let value = match horizontal {
                HorizontalAlignment::General => None,
                HorizontalAlignment::Left => Some("left"),
                HorizontalAlignment::Center => Some("center"),
                HorizontalAlignment::Right => Some("right"),
                HorizontalAlignment::Fill => Some("fill"),
                HorizontalAlignment::Justify => Some("justify"),
                HorizontalAlignment::CenterContinuous => Some("centerContinuous"),
                HorizontalAlignment::Distributed => Some("distributed"),
            };
            match value {
                Some(value) => set_attribute(&mut alignment, "horizontal", value),
                None => alignment.retain(|(key, _)| key != "horizontal"),
            }
        }
        if let Some(vertical) = change.vertical {
            let value = match vertical {
                VerticalAlignment::Bottom => None,
                VerticalAlignment::Top => Some("top"),
                VerticalAlignment::Center => Some("center"),
                VerticalAlignment::Justify => Some("justify"),
                VerticalAlignment::Distributed => Some("distributed"),
            };
            match value {
                Some(value) => set_attribute(&mut alignment, "vertical", value),
                None => alignment.retain(|(key, _)| key != "vertical"),
            }
        }
        if let Some(wrap) = change.wrap_text {
            if wrap {
                set_attribute(&mut alignment, "wrapText", "1");
            } else {
                alignment.retain(|(key, _)| key != "wrapText");
            }
        }
        if let Some(shrink) = change.shrink_to_fit {
            if shrink {
                set_attribute(&mut alignment, "shrinkToFit", "1");
            } else {
                alignment.retain(|(key, _)| key != "shrinkToFit");
            }
        }
        if let Some(rotation) = change.text_rotation {
            if rotation == 0 {
                alignment.retain(|(key, _)| key != "textRotation");
            } else {
                set_attribute(&mut alignment, "textRotation", &rotation.to_string());
            }
        }
        if let Some(indent) = change.indent {
            if indent == 0 {
                alignment.retain(|(key, _)| key != "indent");
            } else {
                set_attribute(&mut alignment, "indent", &indent.to_string());
            }
        }
        // Excel keeps an indent only where it means something: on left,
        // right and distributed text. Indenting General text makes it
        // left-aligned, as the Increase Indent button does.
        if change.indent.is_some_and(|indent| indent > 0)
            && !alignment.iter().any(|(key, _)| key == "horizontal")
        {
            set_attribute(&mut alignment, "horizontal", "left");
        }
        let indents = alignment.iter().any(|(key, value)| {
            key == "horizontal" && matches!(value.as_str(), "left" | "right" | "distributed")
        });
        if !indents {
            alignment.retain(|(key, _)| key != "indent");
        }
        set_attribute(&mut xf_attributes, "applyAlignment", "1");
    }

    if change.locked.is_some() || change.formula_hidden.is_some() {
        let parsed = jade_xlsx_reader::styles::parse_styles(styles_xml)?;
        let current = parsed
            .cell_formats
            .get(base_index)
            .copied()
            .unwrap_or_default();
        let containers = crate::xml_edit_ns::children(styles_xml, b"cellXfs")?;
        let context = containers.first().ok_or(WriteError::NoStyleTables)?;
        let formats = crate::xml_edit_ns::children(&context.2.text, b"xf")?;
        let context = &formats
            .get(base_index)
            .ok_or(WriteError::NoSuchStyle {
                index: base_index as u32,
            })?
            .2;
        let updates = [
            (
                "locked",
                Some(
                    if change.locked.unwrap_or(!current.unlocked) {
                        "1"
                    } else {
                        "0"
                    }
                    .into(),
                ),
            ),
            (
                "hidden",
                Some(
                    if change.formula_hidden.unwrap_or(current.formula_hidden) {
                        "1"
                    } else {
                        "0"
                    }
                    .into(),
                ),
            ),
        ];
        let updated =
            crate::xml_edit_ns::edit_child(&context.text, b"protection", &[b"extLst"], |child| {
                crate::xml_edit::root_attributes(child, &updates)
            })?;
        other_children = children_except(&context.finish(&updated)?, &["alignment"])?;
        set_attribute(&mut xf_attributes, "applyProtection", "1");
    }

    // ── the xf itself ───────────────────────────────────────────────
    let mut xf_text = format!("<{}xf", layout.prefix);
    for (key, value) in &xf_attributes {
        xf_text.push(' ');
        xf_text.push_str(key);
        xf_text.push_str("=\"");
        xf_text.push_str(&escape(value));
        xf_text.push('"');
    }
    let mut children = String::new();
    if !alignment.is_empty() {
        children.push_str(&format!("<{}alignment", layout.prefix));
        for (key, value) in &alignment {
            children.push(' ');
            children.push_str(key);
            children.push_str("=\"");
            children.push_str(&escape(value));
            children.push('"');
        }
        children.push_str("/>");
    }
    for child in &other_children {
        children.push_str(child);
    }
    if children.is_empty() {
        xf_text.push_str("/>");
    } else {
        xf_text.push('>');
        xf_text.push_str(&children);
        xf_text.push_str(&format!("</{}xf>", layout.prefix));
    }

    if let Some(index) = find_entry(styles_xml, &layout.cell_xfs, &xf_text) {
        // Usually nothing is pending then; a border list just created can
        // still be, when an xf already pointed past the missing list.
        return Ok((apply(styles_xml, splices), index));
    }
    let index = append_entry(styles_xml, &layout.cell_xfs, &xf_text, &mut splices)?;
    Ok((apply(styles_xml, splices), index))
}

/// Imports a historical cell format, remapping its component and custom number
/// format IDs. Themes and named-style bases belong to the same workbook.
pub fn import_cell_format(
    target: &str,
    snapshot: &str,
    source_index: u32,
) -> Result<(String, u32)> {
    let source = scan(snapshot)?;
    let xf = source
        .cell_xfs
        .entries
        .get(source_index as usize)
        .ok_or(WriteError::NoSuchStyle {
            index: source_index,
        })?;
    let mut xf_xml = snapshot[xf.start..xf.end].to_string();
    let attributes = attributes_of(&xf_xml)?;
    let mut target = target.to_string();
    for key in ["fontId", "fillId", "borderId"] {
        let Some(index) = attribute_number(&attributes, key) else {
            continue;
        };
        let source_list = match key {
            "fontId" => &source.fonts,
            "fillId" => &source.fills,
            _ => &source.borders,
        };
        let entry = source_list
            .entries
            .get(index as usize)
            .ok_or(WriteError::NoSuchStyle { index })?;
        let xml = contextual_entry(&snapshot[entry.start..entry.end], snapshot)?;
        let layout = scan(&target)?;
        let target_list = match key {
            "fontId" => &layout.fonts,
            "fillId" => &layout.fills,
            _ => &layout.borders,
        };
        let mapped = find_import_entry(&target, target_list, &xml)?;
        let mapped = if let Some(index) = mapped {
            index
        } else {
            let mut splices = Vec::new();
            let index = append_entry(&target, target_list, &xml, &mut splices)?;
            target = apply(&target, splices);
            index
        };
        xf_xml = rewrite_head_and_tail(&xf_xml, key, &mapped.to_string())?;
    }
    if let Some(id) = attribute_number(&attributes, "numFmtId").filter(|id| *id >= 164) {
        let code = source
            .num_fmts
            .entries
            .iter()
            .find_map(|entry| {
                let attrs = attributes_of(&snapshot[entry.start..entry.end]).ok()?;
                (attribute_number(&attrs, "numFmtId") == Some(id))
                    .then(|| {
                        attrs
                            .into_iter()
                            .find(|(key, _)| key == "formatCode")
                            .map(|(_, code)| code)
                    })
                    .flatten()
            })
            .ok_or(WriteError::NoSuchStyle { index: id })?;
        let layout = scan(&target)?;
        let mut splices = Vec::new();
        let mapped = number_format_id(&target, &layout, &code, &mut splices)?;
        target = apply(&target, splices);
        xf_xml = rewrite_head_and_tail(&xf_xml, "numFmtId", &mapped.to_string())?;
    }
    let xml = contextual_entry(&xf_xml, snapshot)?;
    let layout = scan(&target)?;
    if let Some(index) = find_import_entry(&target, &layout.cell_xfs, &xml)? {
        return Ok((target, index));
    }
    let mut splices = Vec::new();
    let index = append_entry(&target, &layout.cell_xfs, &xml, &mut splices)?;
    Ok((apply(&target, splices), index))
}

fn rewrite_head_and_tail(xml: &str, key: &str, value: &str) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    let event = reader.read_event()?;
    let empty = matches!(event, Event::Empty(_));
    let end = reader.buffer_position() as usize;
    match event {
        Event::Start(element) | Event::Empty(element) => Ok(format!(
            "{}{}",
            element_head_with_attribute(&element, key.as_bytes(), value, empty)?,
            &xml[end..]
        )),
        _ => Err(WriteError::NoStyleTables),
    }
}

/// Inherited namespace bindings travel with the entry, so snapshots with a
/// stylesheet prefix or extension attributes remain independently well formed.
fn contextual_entry(entry: &str, styles_xml: &str) -> Result<String> {
    let mut reader = Reader::from_str(styles_xml);
    let mut namespaces = Vec::new();
    loop {
        match reader.read_event()? {
            Event::Start(element) | Event::Empty(element) => {
                for attr in element.attributes() {
                    let attr = attr?;
                    if attr.key.as_ref() == b"xmlns" || attr.key.as_ref().starts_with(b"xmlns:") {
                        namespaces.push((
                            String::from_utf8_lossy(attr.key.as_ref()).into_owned(),
                            attr.unescape_value()?.into_owned(),
                        ));
                    }
                }
                break;
            }
            Event::Eof => return Err(WriteError::NoStyleTables),
            _ => {}
        }
    }
    let mut reader = Reader::from_str(entry);
    let event = reader.read_event()?;
    let empty = matches!(event, Event::Empty(_));
    let end = reader.buffer_position() as usize;
    let element = match event {
        Event::Start(element) | Event::Empty(element) => element,
        _ => return Err(WriteError::NoStyleTables),
    };
    let attrs = attributes_of(entry)?;
    let mut head = entry[..end]
        .trim_end_matches("/>")
        .trim_end_matches('>')
        .to_string();
    for (key, value) in namespaces {
        if !attrs.iter().any(|(name, _)| *name == key) {
            head.push_str(&format!(" {key}=\"{}\"", escape(&value)));
        }
    }
    head.push_str(if empty { "/>" } else { ">" });
    let _ = element;
    Ok(format!("{head}{}", &entry[end..]))
}

fn find_import_entry(xml: &str, list: &List, candidate: &str) -> Result<Option<u32>> {
    let wanted = entry_fingerprint(candidate)?;
    for (index, entry) in list.entries.iter().enumerate() {
        let contextual = contextual_entry(&xml[entry.start..entry.end], xml)?;
        if entry_fingerprint(&contextual)? == wanted {
            return Ok(Some(index as u32));
        }
    }
    Ok(None)
}

/// Expanded names and sorted attributes make reuse independent of prefixes,
/// namespace declaration placement and attribute order.
fn entry_fingerprint(xml: &str) -> Result<String> {
    use quick_xml::{name::ResolveResult, NsReader};
    let expanded = |namespace: ResolveResult<'_>, local: &[u8]| -> String {
        let namespace = match namespace {
            ResolveResult::Bound(namespace) => {
                String::from_utf8_lossy(namespace.as_ref()).into_owned()
            }
            ResolveResult::Unknown(prefix) => format!("?{}", String::from_utf8_lossy(&prefix)),
            ResolveResult::Unbound => String::new(),
        };
        format!("{namespace}|{}", String::from_utf8_lossy(local))
    };
    let mut reader = NsReader::from_str(xml);
    let mut fingerprint = String::new();
    loop {
        let (namespace, event) = reader.read_resolved_event()?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = expanded(namespace, local(element.name().as_ref()));
                let mut attrs = Vec::new();
                for attr in element.attributes() {
                    let attr = attr?;
                    if attr.key.as_ref() == b"xmlns" || attr.key.as_ref().starts_with(b"xmlns:") {
                        continue;
                    }
                    let (namespace, name) = reader.resolver().resolve_attribute(attr.key);
                    attrs.push((
                        expanded(namespace, name.as_ref()),
                        attr.unescape_value()?.into_owned(),
                    ));
                }
                attrs.sort();
                fingerprint.push_str(&format!("<{name}{attrs:?}>"));
                if empty {
                    fingerprint.push_str(&format!("</{name}>"));
                }
            }
            Event::End(element) => fingerprint.push_str(&format!(
                "</{}>",
                expanded(namespace, local(element.name().as_ref()))
            )),
            Event::Text(text) => {
                let text = text.decode()?;
                if !text.trim().is_empty() {
                    fingerprint.push_str(&format!("{text:?}"));
                }
            }
            Event::GeneralRef(entity) => fingerprint.push_str(&format!("&{};", entity.decode()?)),
            Event::CData(text) => fingerprint.push_str(&format!("{:?}", text.decode()?)),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(fingerprint)
}

/// Where everything is in the part.
fn scan(xml: &str) -> Result<Layout> {
    let mut layout = Layout::default();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut section: Option<&'static str> = None;
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name().as_ref().to_vec();
                let local_name = local(&name).to_vec();
                let end = reader.buffer_position() as usize;
                match (section, local_name.as_slice()) {
                    (None, b"numFmts") => {
                        layout.num_fmts.open = Some(Found { start, end, empty });
                        if !empty {
                            section = Some("numFmts");
                        }
                    }
                    (None, b"fonts") => {
                        layout.fonts_at = Some(start);
                        layout.prefix = namespace_prefix(&name);
                        layout.fonts.open = Some(Found { start, end, empty });
                        if !empty {
                            section = Some("fonts");
                        }
                    }
                    (None, b"fills") => {
                        layout.fills.open = Some(Found { start, end, empty });
                        if empty {
                            layout.fills_end = Some(end);
                        } else {
                            section = Some("fills");
                        }
                    }
                    (None, b"borders") => {
                        layout.borders.open = Some(Found { start, end, empty });
                        if !empty {
                            section = Some("borders");
                        }
                    }
                    (None, b"cellXfs") => {
                        layout.cell_xfs.open = Some(Found { start, end, empty });
                        if !empty {
                            section = Some("cellXfs");
                        }
                    }
                    (Some(list), entry)
                        if matches!(
                            (list, entry),
                            ("numFmts", b"numFmt")
                                | ("fonts", b"font")
                                | ("fills", b"fill")
                                | ("borders", b"border")
                                | ("cellXfs", b"xf")
                        ) =>
                    {
                        if !empty {
                            reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                        }
                        let end = reader.buffer_position() as usize;
                        let found = Found { start, end, empty };
                        match list {
                            "numFmts" => layout.num_fmts.entries.push(found),
                            "fonts" => layout.fonts.entries.push(found),
                            "fills" => layout.fills.entries.push(found),
                            "borders" => layout.borders.entries.push(found),
                            _ => layout.cell_xfs.entries.push(found),
                        }
                    }
                    (Some(_), _) if !empty => {
                        // Something unexpected inside a list: skip it whole.
                        reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                    }
                    _ => {}
                }
            }
            Event::End(ref element) => {
                let closing = local(element.name().as_ref()).to_vec();
                match (section, closing.as_slice()) {
                    (Some("numFmts"), b"numFmts") => {
                        layout.num_fmts.close_at = Some(start);
                        section = None;
                    }
                    (Some("fonts"), b"fonts") => {
                        layout.fonts.close_at = Some(start);
                        section = None;
                    }
                    (Some("fills"), b"fills") => {
                        layout.fills.close_at = Some(start);
                        layout.fills_end = Some(reader.buffer_position() as usize);
                        section = None;
                    }
                    (Some("borders"), b"borders") => {
                        layout.borders.close_at = Some(start);
                        section = None;
                    }
                    (Some("cellXfs"), b"cellXfs") => {
                        layout.cell_xfs.close_at = Some(start);
                        section = None;
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if layout.fonts.open.is_none() || layout.cell_xfs.open.is_none() || layout.fills.open.is_none()
    {
        return Err(WriteError::NoStyleTables);
    }
    Ok(layout)
}

/// The index of an entry whose text, whitespace between tags aside, is
/// `candidate`.
fn find_entry(xml: &str, list: &List, candidate: &str) -> Option<u32> {
    let wanted = squeeze(candidate);
    list.entries
        .iter()
        .position(|entry| squeeze(&xml[entry.start..entry.end]) == wanted)
        .map(|index| index as u32)
}

/// Appends `entry` to `list` — reopening a `<fonts/>` written empty — and
/// bumps its `count`. Returns the new entry's index.
fn append_entry(xml: &str, list: &List, entry: &str, splices: &mut Vec<Splice>) -> Result<u32> {
    let open = list.open.as_ref().ok_or(WriteError::NoStyleTables)?;
    let count = list.entries.len() as u32 + 1;
    let head = &xml[open.start..open.end];
    let name = element_name(head);
    if open.empty {
        splices.push(Splice {
            start: open.start,
            end: open.end,
            text: format!(
                "{}{entry}</{name}>",
                rewrite_head(head, "count", &count.to_string(), false)?
            ),
        });
    } else {
        splices.push(Splice {
            start: open.start,
            end: open.end,
            text: rewrite_head(head, "count", &count.to_string(), false)?,
        });
        let close_at = list.close_at.ok_or(WriteError::NoStyleTables)?;
        splices.push(Splice {
            start: close_at,
            end: close_at,
            text: entry.to_string(),
        });
    }
    Ok(count - 1)
}

/// The id `code` has — built in, already in `<numFmts>`, or newly added.
fn number_format_id(
    xml: &str,
    layout: &Layout,
    code: &str,
    splices: &mut Vec<Splice>,
) -> Result<u32> {
    if code.is_empty() || code.eq_ignore_ascii_case("General") {
        return Ok(0);
    }
    if let Some(id) = (0..50).find(|id| builtin_number_format(*id) == Some(code)) {
        return Ok(id);
    }
    let mut highest = 163;
    for entry in &layout.num_fmts.entries {
        let attributes = attributes_of(&xml[entry.start..entry.end])?;
        let id = attribute_number(&attributes, "numFmtId");
        let existing = attributes
            .iter()
            .find(|(key, _)| key == "formatCode")
            .map(|(_, value)| value.as_str());
        if existing == Some(code) {
            if let Some(id) = id {
                return Ok(id);
            }
        }
        highest = highest.max(id.unwrap_or(0));
    }
    let id = highest + 1;
    let entry = format!(
        "<{p}numFmt numFmtId=\"{id}\" formatCode=\"{}\"/>",
        escape(code),
        p = layout.prefix
    );
    if layout.num_fmts.open.is_some() {
        append_entry(xml, &layout.num_fmts, &entry, splices)?;
    } else {
        let at = layout.fonts_at.ok_or(WriteError::NoStyleTables)?;
        splices.push(Splice {
            start: at,
            end: at,
            text: format!(
                "<{p}numFmts count=\"1\">{entry}</{p}numFmts>",
                p = layout.prefix
            ),
        });
    }
    Ok(id)
}

/// `base_font` with `change` applied: the flags and colour it names replaced,
/// everything else — size, name, family, scheme — kept as written.
fn derive_font(base_font: &str, change: &FormatChange, prefix: &str) -> Result<String> {
    let base = children_except(base_font, &[])?;
    let mut bold = None;
    let mut italic = None;
    let mut strike = None;
    let mut underline = None;
    let mut size = None;
    let mut color = None;
    let mut name = None;
    let mut rest = Vec::new();
    for child in base {
        match element_local_name(&child) {
            "b" => bold = Some(child),
            "i" => italic = Some(child),
            "strike" => strike = Some(child),
            "u" => underline = Some(child),
            "sz" => size = Some(child),
            "color" => color = Some(child),
            "name" => name = Some(child),
            _ => rest.push(child),
        }
    }
    let flag = |current: Option<String>, wanted: Option<bool>, tag: &str| match wanted {
        Some(true) => Some(format!("<{prefix}{tag}/>")),
        Some(false) => None,
        None => current.filter(|text| !flag_is_off(text)),
    };
    let bold = flag(bold, change.bold, "b");
    let italic = flag(italic, change.italic, "i");
    let strike = flag(strike, change.strike, "strike");
    let underline = flag(underline, change.underline, "u");
    let size = match change.font_size {
        Some(points) => Some(format!("<{prefix}sz val=\"{}\"/>", trim_number(points))),
        None => size,
    };
    let color = match &change.font_color {
        Some(color) => Some(format!("<{prefix}color {}/>", color_attributes(color)?)),
        None => color,
    };
    let name = match &change.font_name {
        Some(family) => Some(format!("<{prefix}name val=\"{}\"/>", escape(family))),
        None => name,
    };
    let mut out = format!("<{prefix}font>");
    for child in [bold, italic, strike, underline, size, color, name]
        .into_iter()
        .flatten()
    {
        out.push_str(&child);
    }
    for child in rest {
        out.push_str(&child);
    }
    out.push_str(&format!("</{prefix}font>"));
    Ok(out)
}

/// `base_border` (`None`: no border at all) with the edges and diagonal
/// directions `change` names replaced. Edges it does not name — their
/// colours (theme or indexed) included — and anything else the entry holds
/// are kept as written, so a change that alters nothing gives back an entry
/// [`find_entry`] recognises. A removed edge is written `<left/>`, as Excel
/// writes every unset edge.
fn derive_border(base_border: Option<&str>, change: &FormatChange, prefix: &str) -> Result<String> {
    let base = base_border.unwrap_or("<border/>");
    let mut attributes = attributes_of(base)?;
    let mut edges: [(&str, Option<String>); 9] = [
        ("start", None),
        ("end", None),
        ("left", None),
        ("right", None),
        ("top", None),
        ("bottom", None),
        ("diagonal", None),
        ("vertical", None),
        ("horizontal", None),
    ];
    let mut rest = Vec::new();
    for child in children_except(base, &[])? {
        match edges
            .iter_mut()
            .find(|(name, _)| *name == element_local_name(&child))
        {
            Some((_, slot)) => *slot = Some(child),
            None => rest.push(child),
        }
    }
    if base_border.is_none() {
        for (name, slot) in edges.iter_mut().take(7).skip(2) {
            *slot = Some(format!("<{prefix}{name}/>"));
        }
    }
    let wanted = [
        ("left", "start", &change.border_left),
        ("right", "end", &change.border_right),
        ("top", "", &change.border_top),
        ("bottom", "", &change.border_bottom),
        ("diagonal", "", &change.border_diagonal),
    ];
    for (name, alias, line) in wanted {
        let Some(line) = line else {
            continue;
        };
        let text = if line.is_none() {
            format!("<{prefix}{name}/>")
        } else {
            let style = line.style.as_str();
            match &line.color {
                Some(color) => format!(
                    "<{prefix}{name} style=\"{style}\"><{prefix}color {}/></{prefix}{name}>",
                    color_attributes(color)?
                ),
                None => format!("<{prefix}{name} style=\"{style}\"/>"),
            }
        };
        for (slot_name, slot) in edges.iter_mut() {
            if *slot_name == name {
                *slot = Some(text.clone());
            } else if *slot_name == alias {
                // `start`/`end` are the bidi spellings of left/right.
                *slot = None;
            }
        }
    }
    for (key, wanted) in [
        ("diagonalUp", change.diagonal_up),
        ("diagonalDown", change.diagonal_down),
    ] {
        match wanted {
            Some(true) => set_attribute(&mut attributes, key, "1"),
            Some(false) => attributes.retain(|(existing, _)| existing != key),
            None => {}
        }
    }
    let mut out = format!("<{prefix}border");
    for (key, value) in &attributes {
        out.push_str(&format!(" {key}=\"{}\"", escape(value)));
    }
    let children: String = edges
        .into_iter()
        .filter_map(|(_, child)| child)
        .chain(rest)
        .collect();
    if children.is_empty() {
        out.push_str("/>");
    } else {
        out.push('>');
        out.push_str(&children);
        out.push_str(&format!("</{prefix}border>"));
    }
    Ok(out)
}

/// `<b val="0"/>` is a flag that is off.
fn flag_is_off(text: &str) -> bool {
    attributes_of(text)
        .ok()
        .and_then(|attributes| {
            attributes
                .into_iter()
                .find(|(key, _)| key == "val")
                .map(|(_, value)| value == "0" || value == "false")
        })
        .unwrap_or(false)
}

/// `RRGGBB` (with or without a leading `#`), upper-cased.
fn parse_rgb(text: &str) -> Result<String> {
    let hex = text.trim().trim_start_matches('#');
    let hex = match hex.len() {
        6 => hex,
        8 => &hex[2..],
        _ => {
            return Err(WriteError::BadColor {
                value: text.to_string(),
            })
        }
    };
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(WriteError::BadColor {
            value: text.to_string(),
        });
    }
    Ok(hex.to_ascii_uppercase())
}

/// A colour's attributes as styles.xml writes them: `rgb="FFRRGGBB"` for
/// `RRGGBB`, or `theme="N"` and any `tint` for `theme:N[:tint]`, so a theme
/// colour stays one and follows the workbook's theme.
fn color_attributes(text: &str) -> Result<String> {
    let Some(theme) = text.trim().strip_prefix("theme:") else {
        return Ok(format!("rgb=\"FF{}\"", parse_rgb(text)?));
    };
    let bad = || WriteError::BadColor {
        value: text.to_string(),
    };
    let (index, tint) = theme.split_once(':').unwrap_or((theme, ""));
    let index: u32 = index.parse().map_err(|_| bad())?;
    if index > 11 {
        return Err(bad());
    }
    if tint.is_empty() {
        return Ok(format!("theme=\"{index}\""));
    }
    let value: f64 = tint.parse().map_err(|_| bad())?;
    if !(-1.0..=1.0).contains(&value) {
        return Err(bad());
    }
    Ok(if value == 0.0 {
        format!("theme=\"{index}\"")
    } else {
        format!("theme=\"{index}\" tint=\"{}\"", tint.trim())
    })
}

fn trim_number(value: f32) -> String {
    if value == value.trunc() {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

// ── small XML helpers on element text ───────────────────────────────

/// The attributes of the first element in `text`, in order.
fn attributes_of(text: &str) -> Result<Vec<(String, String)>> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(element) | Event::Empty(element) => {
                let mut out = Vec::new();
                for attribute in element.attributes() {
                    let attribute = attribute?;
                    out.push((
                        String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
                        attribute.unescape_value()?.into_owned(),
                    ));
                }
                return Ok(out);
            }
            Event::Eof => return Ok(Vec::new()),
            _ => {}
        }
    }
}

/// The attributes of the first child called `name` of the element in `text`.
fn child_attributes(text: &str, name: &str) -> Result<Vec<(String, String)>> {
    for child in children_except(text, &[])? {
        if element_local_name(&child) == name {
            return attributes_of(&child);
        }
    }
    Ok(Vec::new())
}

/// The direct children of the element in `text`, as their own text, minus
/// those whose local name is in `skip`.
fn children_except(text: &str, skip: &[&str]) -> Result<Vec<String>> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut out = Vec::new();
    loop {
        let start = reader.buffer_position() as usize;
        let event = reader.read_event_into(&mut buf)?;
        match event {
            Event::Start(ref element) if depth == 0 => {
                depth = 1;
                let _ = element;
            }
            Event::Empty(_) if depth == 0 => return Ok(out),
            Event::Start(ref element) | Event::Empty(ref element) if depth == 1 => {
                let empty = matches!(event, Event::Empty(_));
                let name = element.name().as_ref().to_vec();
                if !empty {
                    reader.read_to_end_into(QName(&name), &mut Vec::new())?;
                }
                let end = reader.buffer_position() as usize;
                let child = &text[start..end];
                if !skip.contains(&element_local_name(child)) {
                    out.push(child.to_string());
                }
            }
            Event::End(_) if depth == 1 => return Ok(out),
            Event::Eof => return Ok(out),
            _ => {}
        }
        buf.clear();
    }
}

/// `fonts` for `<x:fonts count="3">`.
fn element_local_name(text: &str) -> &str {
    let name = element_name(text);
    match name.rfind(':') {
        Some(index) => &name[index + 1..],
        None => name,
    }
}

/// `x:fonts` for `<x:fonts count="3">`.
fn element_name(text: &str) -> &str {
    let inner = text.trim_start_matches('<');
    let end = inner
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(inner.len());
    &inner[..end]
}

/// `head` with attribute `key` set to `value`.
fn rewrite_head(head: &str, key: &str, value: &str, empty: bool) -> Result<String> {
    let mut reader = Reader::from_str(head);
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(element) | Event::Empty(element) => {
                return element_head_with_attribute(&element, key.as_bytes(), value, empty);
            }
            Event::Eof => return Ok(head.to_string()),
            _ => {}
        }
    }
}

fn set_attribute(attributes: &mut Vec<(String, String)>, key: &str, value: &str) {
    match attributes.iter_mut().find(|(existing, _)| existing == key) {
        Some((_, existing)) => *existing = value.to_string(),
        None => attributes.push((key.to_string(), value.to_string())),
    }
}

fn attribute_number(attributes: &[(String, String)], key: &str) -> Option<u32> {
    attributes
        .iter()
        .find(|(existing, _)| existing == key)
        .and_then(|(_, value)| value.parse().ok())
}

/// Whitespace between tags removed, for comparing elements.
fn squeeze(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut between_tags = false;
    for c in text.chars() {
        match c {
            '>' => {
                out.push(c);
                between_tags = true;
            }
            '<' => {
                out.push(c);
                between_tags = false;
            }
            c if between_tags && c.is_whitespace() => {}
            c => out.push(c),
        }
    }
    out
}

/// The `s` attribute helper's general form, kept here for the tests' sake.
#[allow(dead_code)]
pub(crate) fn attribute_of(text: &str, key: &str) -> Result<Option<String>> {
    let mut reader = Reader::from_str(text);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(element) | Event::Empty(element) => {
                return attribute(&element, key.as_bytes());
            }
            Event::Eof => return Ok(None),
            _ => {}
        }
    }
}
