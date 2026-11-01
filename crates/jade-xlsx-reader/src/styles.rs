//! `xl/styles.xml` — the font/fill/border tables and the `cellXfs` entries a
//! cell's `s` attribute indexes into.

mod collections;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{name::ResolveResult, NsReader};

use crate::error::Result;
use crate::model::{
    Alignment, Border, BorderEdge, CellFormat, Color, Fill, Font, GradientFill,
    HorizontalAlignment, StyleTable, VerticalAlignment,
};
use crate::theme::ThemePalette;
use jade_opc::local_name;

/// Which table the parser is currently inside. `<color>` and `<name>` appear
/// under several of them and mean different things, so the section is tracked
/// rather than guessed from the tag.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    NumberFormats,
    Fonts,
    Fills,
    Borders,
    CellXfs,
    StyleXfs,
    CellStyles,
}

/// The legacy indexed palette Excel uses when a workbook declares none of
/// its own (ECMA-376 §18.8.27): `indexed="N"` colours, from files saved by
/// older versions or converted from `.xls`, and number formats' `[ColorN]`.
/// 64 and 65, the system foreground and background, are not in it.
pub const INDEXED_PALETTE: [[u8; 3]; 64] = [
    [0x00, 0x00, 0x00],
    [0xFF, 0xFF, 0xFF],
    [0xFF, 0x00, 0x00],
    [0x00, 0xFF, 0x00],
    [0x00, 0x00, 0xFF],
    [0xFF, 0xFF, 0x00],
    [0xFF, 0x00, 0xFF],
    [0x00, 0xFF, 0xFF],
    [0x00, 0x00, 0x00],
    [0xFF, 0xFF, 0xFF],
    [0xFF, 0x00, 0x00],
    [0x00, 0xFF, 0x00],
    [0x00, 0x00, 0xFF],
    [0xFF, 0xFF, 0x00],
    [0xFF, 0x00, 0xFF],
    [0x00, 0xFF, 0xFF],
    [0x80, 0x00, 0x00],
    [0x00, 0x80, 0x00],
    [0x00, 0x00, 0x80],
    [0x80, 0x80, 0x00],
    [0x80, 0x00, 0x80],
    [0x00, 0x80, 0x80],
    [0xC0, 0xC0, 0xC0],
    [0x80, 0x80, 0x80],
    [0x99, 0x99, 0xFF],
    [0x99, 0x33, 0x66],
    [0xFF, 0xFF, 0xCC],
    [0xCC, 0xFF, 0xFF],
    [0x66, 0x00, 0x66],
    [0xFF, 0x80, 0x80],
    [0x00, 0x66, 0xCC],
    [0xCC, 0xCC, 0xFF],
    [0x00, 0x00, 0x80],
    [0xFF, 0x00, 0xFF],
    [0xFF, 0xFF, 0x00],
    [0x00, 0xFF, 0xFF],
    [0x80, 0x00, 0x80],
    [0x80, 0x00, 0x00],
    [0x00, 0x80, 0x80],
    [0x00, 0x00, 0xFF],
    [0x00, 0xCC, 0xFF],
    [0xCC, 0xFF, 0xFF],
    [0xCC, 0xFF, 0xCC],
    [0xFF, 0xFF, 0x99],
    [0x99, 0xCC, 0xFF],
    [0xFF, 0x99, 0xCC],
    [0xCC, 0x99, 0xFF],
    [0xFF, 0xCC, 0x99],
    [0x33, 0x66, 0xFF],
    [0x33, 0xCC, 0xCC],
    [0x99, 0xCC, 0x00],
    [0xFF, 0xCC, 0x00],
    [0xFF, 0x99, 0x00],
    [0xFF, 0x66, 0x00],
    [0x66, 0x66, 0x99],
    [0x96, 0x96, 0x96],
    [0x00, 0x33, 0x66],
    [0x33, 0x99, 0x66],
    [0x00, 0x33, 0x00],
    [0x33, 0x33, 0x00],
    [0x99, 0x33, 0x00],
    [0x99, 0x33, 0x66],
    [0x33, 0x33, 0x99],
    [0x33, 0x33, 0x33],
];

pub fn parse_styles(xml: &str) -> Result<StyleTable> {
    let mut reader = NsReader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut table = StyleTable::default();
    let mut section = Section::None;
    let mut normal_builtin_seen = false;
    let mut font: Option<Font> = None;
    let mut fill: Option<Fill> = None;
    let mut in_pattern_fill = false;
    // The position of the gradient `<stop>` whose colour comes next.
    let mut gradient_stop: Option<f64> = None;
    let mut border: Option<Border> = None;
    let mut border_edge: Option<(&'static str, BorderEdge)> = None;
    let mut format: Option<CellFormat> = None;
    let mut buf = Vec::new();

    loop {
        let (namespace, event) = reader.read_resolved_event_into(&mut buf)?;
        let main = match namespace {
            ResolveResult::Unbound => true,
            ResolveResult::Bound(namespace) => matches!(
                namespace.as_ref(),
                b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                    | b"http://purl.oclc.org/ooxml/spreadsheetml/main"
            ),
            ResolveResult::Unknown(_) => false,
        };
        if !main {
            buf.clear();
            continue;
        }
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let empty = matches!(event, Event::Empty(_));
                let name = local_name(element.name().as_ref()).to_vec();
                match name.as_slice() {
                    b"numFmts" => section = Section::NumberFormats,
                    b"numFmt" if section == Section::NumberFormats => {
                        let mut id = None;
                        let mut code = None;
                        for attribute in element.attributes() {
                            let attribute = attribute?;
                            match attribute.key.as_ref() {
                                b"numFmtId" => id = attribute.unescape_value()?.parse::<u32>().ok(),
                                b"formatCode" => {
                                    code = Some(attribute.unescape_value()?.into_owned())
                                }
                                _ => {}
                            }
                        }
                        if let (Some(id), Some(code)) = (id, code) {
                            table.number_formats.insert(id, code);
                        }
                    }
                    b"fonts" => section = Section::Fonts,
                    b"fills" => section = Section::Fills,
                    b"borders" => section = Section::Borders,
                    b"cellXfs" => section = Section::CellXfs,
                    b"cellStyleXfs" => section = Section::StyleXfs,
                    b"cellStyles" => section = Section::CellStyles,
                    b"cellStyle" if section == Section::CellStyles => {
                        let builtin = attribute_value(element, b"builtinId")?;
                        let name = attribute_value(element, b"name")?;
                        let built_in_normal = builtin
                            .as_deref()
                            .and_then(|value| value.parse::<u32>().ok())
                            == Some(0);
                        let named_normal = !normal_builtin_seen
                            && builtin.is_none()
                            && name
                                .as_deref()
                                .is_some_and(|name| name.eq_ignore_ascii_case("Normal"));
                        if built_in_normal || named_normal {
                            table.normal_style_index = attribute_value(element, b"xfId")?
                                .and_then(|value| value.parse().ok());
                            normal_builtin_seen |= built_in_normal;
                        }
                    }
                    b"font" if section == Section::Fonts => {
                        font = Some(Font::default());
                        if empty {
                            table.fonts.push(font.take().unwrap_or_default());
                        }
                    }
                    b"fill" if section == Section::Fills => {
                        fill = Some(Fill::default());
                        if empty {
                            table.fills.push(fill.take().unwrap_or_default());
                        }
                    }
                    b"border" if section == Section::Borders => {
                        let mut parsed = Border::default();
                        for attribute in element.attributes() {
                            let attribute = attribute?;
                            let value = attribute.unescape_value()?;
                            let enabled = value != "0" && value != "false";
                            match attribute.key.as_ref() {
                                b"diagonalUp" => parsed.diagonal_up = enabled,
                                b"diagonalDown" => parsed.diagonal_down = enabled,
                                _ => {}
                            }
                        }
                        border = Some(parsed);
                        if empty {
                            table.borders.push(border.take().unwrap_or_default());
                        }
                    }
                    b"xf" if matches!(section, Section::CellXfs | Section::StyleXfs) => {
                        let mut entry = CellFormat::default();
                        for attribute in element.attributes() {
                            let attribute = attribute?;
                            let value = attribute.unescape_value()?;
                            match attribute.key.as_ref() {
                                b"fontId" => entry.font_index = value.parse().ok(),
                                b"applyFont" => {
                                    entry.apply_font = match value.as_ref() {
                                        "1" | "true" => Some(true),
                                        "0" | "false" => Some(false),
                                        _ => None,
                                    }
                                }
                                b"fillId" => entry.fill_index = value.parse().ok(),
                                b"borderId" => entry.border_index = value.parse().ok(),
                                b"numFmtId" => entry.number_format_id = value.parse().ok(),
                                _ => {}
                            }
                        }
                        if empty {
                            if section == Section::StyleXfs {
                                table.style_formats.push(entry);
                            } else {
                                table.cell_formats.push(entry);
                            }
                        } else {
                            format = Some(entry);
                        }
                    }
                    b"alignment" => {
                        if let Some(entry) = format.as_mut() {
                            entry.alignment = parse_alignment(element)?;
                        }
                    }
                    b"protection" => {
                        if let Some(entry) = format.as_mut() {
                            entry.unlocked = attribute_value(element, b"locked")?
                                .is_some_and(|value| value == "0" || value == "false");
                        }
                    }
                    // ---- font children ----
                    b"b" if section == Section::Fonts => {
                        set_font_flag(&mut font, element, |f| &mut f.bold)?
                    }
                    b"i" if section == Section::Fonts => {
                        set_font_flag(&mut font, element, |f| &mut f.italic)?
                    }
                    b"u" if section == Section::Fonts => {
                        set_font_flag(&mut font, element, |f| &mut f.underline)?
                    }
                    b"strike" if section == Section::Fonts => {
                        set_font_flag(&mut font, element, |f| &mut f.strike)?
                    }
                    b"vertAlign" if section == Section::Fonts => {
                        if let Some(font) = font.as_mut() {
                            font.vert_align = attribute_value(element, b"val")?
                                .filter(|value| value != "baseline");
                        }
                    }
                    b"sz" if section == Section::Fonts => {
                        if let Some(font) = font.as_mut() {
                            font.size_points = attribute_value(element, b"val")?
                                .and_then(|value| value.parse().ok());
                        }
                    }
                    b"name" | b"rFont" if section == Section::Fonts => {
                        if let Some(font) = font.as_mut() {
                            font.name = attribute_value(element, b"val")?;
                        }
                    }
                    b"color" => match section {
                        Section::Fonts => {
                            if let Some(font) = font.as_mut() {
                                font.color = parse_color(element)?;
                            }
                        }
                        Section::Borders => {
                            if let Some((_, edge)) = border_edge.as_mut() {
                                edge.color = parse_color(element)?;
                            }
                        }
                        // A gradient `<stop>`'s colour.
                        Section::Fills => {
                            if let (Some(position), Some(color), Some(gradient)) = (
                                gradient_stop.take(),
                                parse_color(element)?,
                                fill.as_mut().and_then(|fill| fill.gradient.as_mut()),
                            ) {
                                gradient.stops.push((position, color));
                            }
                        }
                        _ => {}
                    },
                    // ---- fill children ----
                    b"patternFill" if section == Section::Fills => {
                        in_pattern_fill = true;
                        if let Some(fill) = fill.as_mut() {
                            fill.pattern = attribute_value(element, b"patternType")?;
                        }
                    }
                    b"gradientFill" if section == Section::Fills => {
                        if let Some(fill) = fill.as_mut() {
                            let number = |value: Option<String>| {
                                value
                                    .and_then(|value| value.parse::<f64>().ok())
                                    .filter(|value| value.is_finite())
                            };
                            let path =
                                attribute_value(element, b"type")?.as_deref() == Some("path");
                            let edge = |name: &[u8]| -> Result<f64> {
                                Ok(number(attribute_value(element, name)?)
                                    .unwrap_or(0.0)
                                    .clamp(0.0, 1.0))
                            };
                            fill.gradient = Some(GradientFill {
                                degree: number(attribute_value(element, b"degree")?).unwrap_or(0.0),
                                path: if path {
                                    Some([
                                        edge(b"left")?,
                                        edge(b"right")?,
                                        edge(b"top")?,
                                        edge(b"bottom")?,
                                    ])
                                } else {
                                    None
                                },
                                stops: Vec::new(),
                            });
                        }
                    }
                    b"stop" if section == Section::Fills => {
                        gradient_stop = attribute_value(element, b"position")?
                            .and_then(|value| value.parse::<f64>().ok())
                            .filter(|value| value.is_finite())
                            .map(|value| value.clamp(0.0, 1.0));
                    }
                    b"fgColor" if in_pattern_fill => {
                        if let Some(fill) = fill.as_mut() {
                            fill.foreground = parse_color(element)?;
                        }
                    }
                    b"bgColor" if in_pattern_fill => {
                        if let Some(fill) = fill.as_mut() {
                            fill.background = parse_color(element)?;
                        }
                    }
                    // ---- border children ----
                    b"left" | b"right" | b"top" | b"bottom" | b"diagonal"
                        if section == Section::Borders =>
                    {
                        let side = match name.as_slice() {
                            b"left" => "left",
                            b"right" => "right",
                            b"top" => "top",
                            b"bottom" => "bottom",
                            _ => "diagonal",
                        };
                        let style = attribute_value(element, b"style")?;
                        // A side with no `style` is "no border": Excel writes
                        // `<left/>` for every unset side of every border entry.
                        let edge = style.map(|style| BorderEdge { style, color: None });
                        match (empty, edge) {
                            (_, None) => border_edge = None,
                            (true, Some(edge)) => set_border_edge(&mut border, side, edge),
                            (false, Some(edge)) => border_edge = Some((side, edge)),
                        }
                    }
                    _ => {}
                }
            }
            Event::End(ref element) => match local_name(element.name().as_ref()) {
                b"font" => {
                    if let Some(font) = font.take() {
                        table.fonts.push(font);
                    }
                }
                b"fill" => {
                    if let Some(fill) = fill.take() {
                        table.fills.push(fill);
                    }
                }
                b"patternFill" => in_pattern_fill = false,
                b"left" | b"right" | b"top" | b"bottom" | b"diagonal" => {
                    if let Some((side, edge)) = border_edge.take() {
                        set_border_edge(&mut border, side, edge);
                    }
                }
                b"border" => {
                    if let Some(border) = border.take() {
                        table.borders.push(border);
                    }
                }
                b"xf" => {
                    if let Some(entry) = format.take() {
                        if section == Section::StyleXfs {
                            table.style_formats.push(entry);
                        } else {
                            table.cell_formats.push(entry);
                        }
                    }
                }
                b"numFmts" | b"fonts" | b"fills" | b"borders" | b"cellXfs" | b"cellStyleXfs"
                | b"cellStyles" => section = Section::None,
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    crate::sheet_protection::style_flags(xml, &mut table)?;
    let collections = collections::parse(xml)?;
    table.differential_styles = collections.differential;
    table.table_styles = collections.tables;
    table.default_pivot_style = collections.default_pivot;
    apply_indexed_colors(&mut table, &indexed_palette(xml)?);
    Ok(table)
}

/// The workbook's own indexed palette (`<colors><indexedColors>`), entry by
/// entry; empty when it keeps Excel's.
fn indexed_palette(xml: &str) -> Result<Vec<[u8; 3]>> {
    let mut palette = Vec::new();
    if !xml.contains("indexedColors") {
        return Ok(palette);
    }
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut inside = false;
    loop {
        match reader.read_event()? {
            Event::Start(ref element) | Event::Empty(ref element) => {
                match local_name(element.name().as_ref()) {
                    b"indexedColors" => inside = true,
                    b"rgbColor" if inside => {
                        let rgb = attribute_value(element, b"rgb")?
                            .and_then(|value| parse_argb(&value))
                            .map(|[_, red, green, blue]| [red, green, blue])
                            .unwrap_or_default();
                        palette.push(rgb);
                    }
                    _ => {}
                }
            }
            Event::End(ref element) if local_name(element.name().as_ref()) == b"indexedColors" => {
                inside = false;
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(palette)
}

/// Every indexed colour in the table but the system ones made the colour
/// `palette` (or else Excel's) gives it.
fn apply_indexed_colors(table: &mut StyleTable, palette: &[[u8; 3]]) {
    each_color(table, &mut |color: &mut Color| {
        if let Color::Indexed(index) = *color {
            let entry = palette
                .get(index as usize)
                .or_else(|| INDEXED_PALETTE.get(index as usize));
            if let Some(&[red, green, blue]) = entry {
                *color = Color::Argb([0xFF, red, green, blue]);
            }
        }
    });
}

/// Calls `visit` on each colour the table's fonts, fills, borders and
/// differential styles hold.
fn each_color(table: &mut StyleTable, visit: &mut dyn FnMut(&mut Color)) {
    let mut resolve = |color: &mut Option<Color>| {
        if let Some(color) = color {
            visit(color);
        }
    };
    for font in &mut table.fonts {
        resolve(&mut font.color);
    }
    for style in &mut table.differential_styles {
        resolve(&mut style.text_color);
        if let Some(fill) = style.fill.as_mut() {
            resolve(&mut fill.foreground);
            resolve(&mut fill.background);
        }
        for edge in [
            &mut style.left,
            &mut style.right,
            &mut style.top,
            &mut style.bottom,
            &mut style.inside_vertical,
            &mut style.inside_horizontal,
        ]
        .into_iter()
        .flatten()
        {
            if let crate::conditional_format::DifferentialBorder::Edge(edge) = edge {
                resolve(&mut edge.color);
            }
        }
    }
    for fill in &mut table.fills {
        resolve(&mut fill.foreground);
        resolve(&mut fill.background);
        for (_, color) in fill
            .gradient
            .iter_mut()
            .flat_map(|gradient| gradient.stops.iter_mut())
        {
            let mut slot = Some(*color);
            resolve(&mut slot);
            *color = slot.unwrap_or(*color);
        }
    }
    for border in &mut table.borders {
        for edge in [
            &mut border.left,
            &mut border.right,
            &mut border.top,
            &mut border.bottom,
            &mut border.diagonal,
        ]
        .into_iter()
        .flatten()
        {
            resolve(&mut edge.color);
        }
    }
}

fn set_border_edge(border: &mut Option<Border>, side: &str, edge: BorderEdge) {
    let Some(border) = border.as_mut() else {
        return;
    };
    match side {
        "left" => border.left = Some(edge),
        "right" => border.right = Some(edge),
        "top" => border.top = Some(edge),
        "bottom" => border.bottom = Some(edge),
        _ => border.diagonal = Some(edge),
    }
}

/// `<b/>` means bold; `<b val="0"/>` means explicitly not bold.
fn set_font_flag(
    font: &mut Option<Font>,
    element: &BytesStart<'_>,
    field: impl Fn(&mut Font) -> &mut bool,
) -> Result<()> {
    let Some(font) = font.as_mut() else {
        return Ok(());
    };
    let enabled = match attribute_value(element, b"val")? {
        None => true,
        Some(value) => value != "0" && value != "false",
    };
    *field(font) = enabled;
    Ok(())
}

fn parse_alignment(element: &BytesStart<'_>) -> Result<Alignment> {
    let mut alignment = Alignment::default();
    for attribute in element.attributes() {
        let attribute = attribute?;
        let value = attribute.unescape_value()?;
        match attribute.key.as_ref() {
            b"horizontal" => {
                alignment.horizontal = match value.as_ref() {
                    "left" => HorizontalAlignment::Left,
                    "center" => HorizontalAlignment::Center,
                    "right" => HorizontalAlignment::Right,
                    "fill" => HorizontalAlignment::Fill,
                    "justify" => HorizontalAlignment::Justify,
                    "centerContinuous" => HorizontalAlignment::CenterContinuous,
                    "distributed" => HorizontalAlignment::Distributed,
                    _ => HorizontalAlignment::General,
                }
            }
            b"vertical" => {
                alignment.vertical = match value.as_ref() {
                    "top" => VerticalAlignment::Top,
                    "center" => VerticalAlignment::Center,
                    "justify" => VerticalAlignment::Justify,
                    "distributed" => VerticalAlignment::Distributed,
                    _ => VerticalAlignment::Bottom,
                }
            }
            b"wrapText" => alignment.wrap_text = value != "0" && value != "false",
            b"shrinkToFit" => alignment.shrink_to_fit = value != "0" && value != "false",
            b"indent" => alignment.indent = value.parse().unwrap_or_default(),
            b"textRotation" => {
                alignment.text_rotation = value
                    .parse::<u16>()
                    .ok()
                    .filter(|rotation| *rotation <= 180 || *rotation == 255)
                    .unwrap_or_default()
            }
            _ => {}
        }
    }
    Ok(alignment)
}

pub(crate) fn parse_color(element: &BytesStart<'_>) -> Result<Option<Color>> {
    let mut color = None;
    let mut tint = 0.0f32;
    let mut theme_index = None;
    for attribute in element.attributes() {
        let attribute = attribute?;
        let value = attribute.unescape_value()?;
        match attribute.key.as_ref() {
            b"rgb" => color = parse_argb(&value).map(Color::Argb),
            b"indexed" => color = value.parse().ok().map(Color::Indexed),
            b"theme" => theme_index = value.parse::<u32>().ok(),
            b"tint" => tint = value.parse().unwrap_or(0.0),
            b"auto" => {
                if value != "0" && value != "false" {
                    color = Some(Color::Auto);
                }
            }
            _ => {}
        }
    }
    if let Some(index) = theme_index {
        color = Some(Color::Theme {
            index,
            tint,
            resolved: None,
        });
    }
    Ok(color)
}

pub(crate) fn apply_theme_colors(table: &mut StyleTable, theme: &ThemePalette) {
    table.theme_colors = (0..12).map(|index| theme.color(index)).collect();
    each_color(table, &mut |color: &mut Color| {
        if let Color::Theme {
            index, resolved, ..
        } = color
        {
            *resolved = theme.color(*index);
        }
    });
}

/// `FFRRGGBB`, and the 6-digit form some writers emit (alpha implied opaque).
fn parse_argb(value: &str) -> Option<[u8; 4]> {
    let bytes = match value.len() {
        8 => value,
        6 => return parse_argb(&format!("FF{value}")),
        _ => return None,
    };
    let mut out = [0u8; 4];
    for (index, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&bytes[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn attribute_value(element: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>> {
    for attribute in element.attributes() {
        let attribute = attribute?;
        if attribute.key.as_ref() == key {
            return Ok(Some(attribute.unescape_value()?.into_owned()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_diagonal_border_directions_and_their_shared_edge() {
        let styles = parse_styles(
            r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
              <borders count="1"><border diagonalUp="1" diagonalDown="true">
                <diagonal style="mediumDashDot"><color rgb="FF008000"/></diagonal>
              </border></borders>
            </styleSheet>"#,
        )
        .expect("styles");
        let border = &styles.borders[0];

        assert!(border.diagonal_up);
        assert!(border.diagonal_down);
        let edge = border.diagonal.as_ref().expect("diagonal edge");
        assert_eq!(edge.style, "mediumDashDot");
        assert_eq!(edge.color, Some(Color::Argb([0xFF, 0x00, 0x80, 0x00])));
    }
}

#[cfg(test)]
mod indexed_tests {
    use super::*;

    #[test]
    fn indexed_colours_take_the_workbooks_palette_or_excels() {
        let xml = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts><font><color indexed="10"/></font><font><color indexed="64"/></font></fonts><fills><fill><patternFill patternType="solid"><fgColor indexed="22"/><bgColor indexed="64"/></patternFill></fill></fills></styleSheet>"#;
        let table = parse_styles(xml).unwrap();
        assert_eq!(table.fonts[0].color, Some(Color::Argb([0xFF, 0xFF, 0, 0])));
        // The system foreground stays what it is.
        assert_eq!(table.fonts[1].color, Some(Color::Indexed(64)));
        assert_eq!(
            table.fills[0].foreground,
            Some(Color::Argb([0xFF, 0xC0, 0xC0, 0xC0]))
        );
        let custom = xml.replace(
            "</styleSheet>",
            r#"<colors><indexedColors><rgbColor rgb="FF000000"/><rgbColor rgb="FF111111"/><rgbColor rgb="FF222222"/><rgbColor rgb="FF333333"/><rgbColor rgb="FF444444"/><rgbColor rgb="FF555555"/><rgbColor rgb="FF666666"/><rgbColor rgb="FF777777"/><rgbColor rgb="FF888888"/><rgbColor rgb="FF999999"/><rgbColor rgb="FF123456"/></indexedColors></colors></styleSheet>"#,
        );
        let table = parse_styles(&custom).unwrap();
        assert_eq!(
            table.fonts[0].color,
            Some(Color::Argb([0xFF, 0x12, 0x34, 0x56]))
        );
        // Past the workbook's palette, Excel's.
        assert_eq!(
            table.fills[0].foreground,
            Some(Color::Argb([0xFF, 0xC0, 0xC0, 0xC0]))
        );
    }
}
