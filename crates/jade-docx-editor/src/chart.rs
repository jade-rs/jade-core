use crate::document::{ChartKind, ChartSeries, RunChart};
use crate::docx_xml::{DocxXmlDocument, XmlElement, XmlNode};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use std::io::Write;
use zip::write::SimpleFileOptions;

pub(crate) const MAX_CHART_CATEGORIES: usize = 64;
pub(crate) const MAX_CHART_SERIES: usize = 16;
pub(crate) const MAX_CHART_LABEL_BYTES: usize = 512;

fn local_name(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

fn attr_value<'a>(element: &'a XmlElement, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|attribute| local_name(&attribute.name) == name)
        .map(|attribute| attribute.value.as_str())
}

fn descendant<'a>(element: &'a XmlElement, name: &str) -> Option<&'a XmlElement> {
    if local_name(&element.name) == name {
        return Some(element);
    }
    element.children.iter().find_map(|child| match child {
        XmlNode::Element(child) => descendant(child, name),
        _ => None,
    })
}

fn descendants<'a>(element: &'a XmlElement, name: &str, output: &mut Vec<&'a XmlElement>) {
    if local_name(&element.name) == name {
        output.push(element);
    }
    for child in &element.children {
        if let XmlNode::Element(child) = child {
            descendants(child, name, output);
        }
    }
}

fn direct_descendants<'a>(element: &'a XmlElement, name: &str) -> Vec<&'a XmlElement> {
    let mut result = Vec::new();
    for child in &element.children {
        if let XmlNode::Element(child) = child {
            if local_name(&child.name) == name {
                result.push(child);
            } else {
                result.extend(direct_descendants(child, name));
            }
        }
    }
    result
}

fn element_text(element: &XmlElement) -> String {
    let mut result = String::new();
    for child in &element.children {
        match child {
            XmlNode::Text(text) | XmlNode::CData(text) => result.push_str(text),
            XmlNode::Element(child) => result.push_str(&element_text(child)),
            XmlNode::Comment(_) => {}
        }
    }
    result
}

fn point_values(container: &XmlElement) -> Vec<(usize, String)> {
    let mut points = Vec::new();
    descendants(container, "pt", &mut points);
    let mut values = points
        .into_iter()
        .filter_map(|point| {
            let index = attr_value(point, "idx")?.parse::<usize>().ok()?;
            let value = descendant(point, "v").map(element_text).unwrap_or_default();
            Some((index, value))
        })
        .collect::<Vec<_>>();
    values.sort_by_key(|(index, _)| *index);
    values.dedup_by_key(|(index, _)| *index);
    values
}

fn point_count(container: &XmlElement) -> usize {
    descendant(container, "ptCount")
        .and_then(|element| attr_value(element, "val"))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_else(|| {
            point_values(container)
                .last()
                .map_or(0, |(index, _)| index.saturating_add(1))
        })
}

fn string_points(container: &XmlElement) -> Vec<String> {
    let count = point_count(container).min(MAX_CHART_CATEGORIES);
    let points = point_values(container);
    let mut values = vec![String::new(); count];
    for (index, value) in points {
        if let Some(slot) = values.get_mut(index) {
            *slot = value;
        }
    }
    values
}

fn number_points(container: &XmlElement, count_hint: usize) -> Vec<Option<f64>> {
    let count = point_count(container)
        .max(count_hint)
        .min(MAX_CHART_CATEGORIES);
    let points = point_values(container);
    let mut values = vec![None; count];
    for (index, value) in points {
        if let Some(slot) = values.get_mut(index) {
            *slot = value.parse::<f64>().ok().filter(|value| value.is_finite());
        }
    }
    values
}

/// Parses the cached values of a native ChartML part. Formula references are
/// intentionally read through their `strCache`/`numCache`, matching what Word
/// displays when an embedded workbook is unavailable.
pub(crate) fn parse_chart_xml(xml: &str) -> Result<RunChart, String> {
    parse_chart_xml_with(xml, false)
}

/// Reads any ChartML plot into the nearest of the three previewable kinds:
/// 3-D bar/line/pie as their 2-D form, doughnut and bar-of-pie as pie, and
/// area, scatter, radar and stock as lines. Only for drawing an existing
/// chart; editing keeps the strict [`parse_chart_xml`] so a chart is never
/// rewritten as another kind.
pub(crate) fn parse_chart_xml_for_preview(xml: &str) -> Result<RunChart, String> {
    parse_chart_xml_with(xml, true)
}

fn parse_chart_xml_with(xml: &str, preview: bool) -> Result<RunChart, String> {
    const STRICT: &[(&str, ChartKind)] = &[
        ("barChart", ChartKind::Bar),
        ("lineChart", ChartKind::Line),
        ("pieChart", ChartKind::Pie),
    ];
    const PREVIEW: &[(&str, ChartKind)] = &[
        ("bar3DChart", ChartKind::Bar),
        ("line3DChart", ChartKind::Line),
        ("pie3DChart", ChartKind::Pie),
        ("doughnutChart", ChartKind::Pie),
        ("ofPieChart", ChartKind::Pie),
        ("areaChart", ChartKind::Line),
        ("area3DChart", ChartKind::Line),
        ("scatterChart", ChartKind::Line),
        ("radarChart", ChartKind::Line),
        ("stockChart", ChartKind::Line),
    ];
    let document = DocxXmlDocument::parse(xml).map_err(|error| error.to_string())?;
    let root = document.root();
    let kinds = STRICT.iter().chain(PREVIEW.iter().filter(|_| preview));
    let Some((kind, plot)) = kinds
        .into_iter()
        .find_map(|(name, kind)| descendant(root, name).map(|plot| (*kind, plot)))
    else {
        return Err("chart kind is not supported (expected bar, line, or pie)".to_string());
    };

    let title = descendant(root, "title")
        .map(|title| {
            let mut text_nodes = Vec::new();
            descendants(title, "t", &mut text_nodes);
            if text_nodes.is_empty() {
                descendant(title, "v").map(element_text).unwrap_or_default()
            } else {
                text_nodes.into_iter().map(element_text).collect::<String>()
            }
        })
        .unwrap_or_default();

    let series_nodes = direct_descendants(plot, "ser");
    let categories = series_nodes
        .first()
        .and_then(|series| {
            descendant(series, "cat")
                .or_else(|| preview.then(|| descendant(series, "xVal")).flatten())
        })
        .map(string_points)
        .unwrap_or_default();
    let mut series = Vec::new();
    for (series_index, series_node) in series_nodes.into_iter().take(MAX_CHART_SERIES).enumerate() {
        let name = descendant(series_node, "tx")
            .and_then(|tx| descendant(tx, "v"))
            .map(element_text)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("Series {}", series_index + 1));
        // Scatter series carry their values as `c:yVal`.
        let values = descendant(series_node, "val")
            .or_else(|| preview.then(|| descendant(series_node, "yVal")).flatten())
            .map(|values| number_points(values, categories.len()))
            .unwrap_or_else(|| vec![None; categories.len()]);
        series.push(ChartSeries { name, values });
    }

    let legend = descendant(root, "legend").map(|legend| {
        descendant(legend, "legendPos")
            .and_then(|position| attr_value(position, "val"))
            .unwrap_or("r")
            .to_string()
    });
    let chart = RunChart {
        kind,
        title,
        categories,
        series,
        legend,
        xml: Some(std::rc::Rc::from(xml)),
    };
    validate_chart(&chart)?;
    Ok(chart)
}

pub(crate) fn validate_chart(chart: &RunChart) -> Result<(), String> {
    if chart.title.len() > MAX_CHART_LABEL_BYTES {
        return Err(format!(
            "chart title exceeds the {MAX_CHART_LABEL_BYTES} byte limit"
        ));
    }
    if chart.categories.is_empty() || chart.categories.len() > MAX_CHART_CATEGORIES {
        return Err(format!(
            "chart must have between 1 and {MAX_CHART_CATEGORIES} categories"
        ));
    }
    if chart.series.is_empty() || chart.series.len() > MAX_CHART_SERIES {
        return Err(format!(
            "chart must have between 1 and {MAX_CHART_SERIES} series"
        ));
    }
    for label in chart
        .categories
        .iter()
        .chain(chart.series.iter().map(|series| &series.name))
    {
        if label.len() > MAX_CHART_LABEL_BYTES {
            return Err(format!(
                "chart label exceeds the {MAX_CHART_LABEL_BYTES} byte limit"
            ));
        }
    }
    for series in &chart.series {
        if series.values.len() != chart.categories.len() {
            return Err("every chart series must have one value per category".to_string());
        }
        if series
            .values
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
        {
            return Err("chart values must be finite numbers or gaps".to_string());
        }
    }
    Ok(())
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xlsx_column(index: usize) -> String {
    let mut value = index + 1;
    let mut result = String::new();
    while value > 0 {
        value -= 1;
        result.insert(0, (b'A' + (value % 26) as u8) as char);
        value /= 26;
    }
    result
}

fn series_xml(index: usize, series: &ChartSeries, categories: &[String]) -> String {
    let categories_xml = categories
        .iter()
        .enumerate()
        .map(|(point, category)| {
            format!(
                "<c:pt idx=\"{point}\"><c:v>{}</c:v></c:pt>",
                escape_xml(category)
            )
        })
        .collect::<String>();
    let values_xml = series
        .values
        .iter()
        .enumerate()
        .filter_map(|(point, value)| {
            value.map(|value| format!("<c:pt idx=\"{point}\"><c:v>{value}</c:v></c:pt>"))
        })
        .collect::<String>();
    let column = xlsx_column(index + 1);
    let last_row = categories.len() + 1;
    format!(
        "<c:ser><c:idx val=\"{index}\"/><c:order val=\"{index}\"/><c:tx><c:strRef><c:f>Sheet1!${column}$1</c:f><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>{}</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:f>Sheet1!$A$2:$A${last_row}</c:f><c:strCache><c:ptCount val=\"{}\"/>{categories_xml}</c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Sheet1!${column}$2:${column}${last_row}</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"{}\"/>{values_xml}</c:numCache></c:numRef></c:val></c:ser>",
        escape_xml(&series.name),
        categories.len(),
        series.values.len()
    )
}

pub(crate) fn build_chart_xml(
    chart: &RunChart,
    external_data_relationship_id: Option<&str>,
) -> Result<String, String> {
    validate_chart(chart)?;
    let series = chart
        .series
        .iter()
        .enumerate()
        .map(|(index, series)| series_xml(index, series, &chart.categories))
        .collect::<String>();
    let plot = match chart.kind {
        ChartKind::Bar => format!(
            "<c:barChart><c:barDir val=\"col\"/><c:grouping val=\"clustered\"/><c:varyColors val=\"0\"/>{series}<c:gapWidth val=\"150\"/><c:axId val=\"48650112\"/><c:axId val=\"48672768\"/></c:barChart><c:catAx><c:axId val=\"48650112\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"b\"/><c:tickLblPos val=\"nextTo\"/><c:crossAx val=\"48672768\"/><c:crosses val=\"autoZero\"/><c:auto val=\"1\"/><c:lblAlgn val=\"ctr\"/><c:lblOffset val=\"100\"/></c:catAx><c:valAx><c:axId val=\"48672768\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"l\"/><c:numFmt formatCode=\"General\" sourceLinked=\"1\"/><c:tickLblPos val=\"nextTo\"/><c:crossAx val=\"48650112\"/><c:crosses val=\"autoZero\"/><c:crossBetween val=\"between\"/></c:valAx>"
        ),
        ChartKind::Line => format!(
            "<c:lineChart><c:grouping val=\"standard\"/><c:varyColors val=\"0\"/>{series}<c:marker val=\"1\"/><c:smooth val=\"0\"/><c:axId val=\"48650112\"/><c:axId val=\"48672768\"/></c:lineChart><c:catAx><c:axId val=\"48650112\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"b\"/><c:tickLblPos val=\"nextTo\"/><c:crossAx val=\"48672768\"/><c:crosses val=\"autoZero\"/><c:auto val=\"1\"/><c:lblAlgn val=\"ctr\"/><c:lblOffset val=\"100\"/></c:catAx><c:valAx><c:axId val=\"48672768\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"l\"/><c:numFmt formatCode=\"General\" sourceLinked=\"1\"/><c:tickLblPos val=\"nextTo\"/><c:crossAx val=\"48650112\"/><c:crosses val=\"autoZero\"/><c:crossBetween val=\"between\"/></c:valAx>"
        ),
        ChartKind::Pie => format!(
            "<c:pieChart><c:varyColors val=\"1\"/>{series}<c:firstSliceAng val=\"0\"/></c:pieChart>"
        ),
    };
    let external_data = external_data_relationship_id
        .map(|relationship_id| {
            format!(
                "<c:externalData r:id=\"{}\"><c:autoUpdate val=\"0\"/></c:externalData>",
                escape_xml(relationship_id)
            )
        })
        .unwrap_or_default();
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><c:date1904 val=\"0\"/><c:lang val=\"ja-JP\"/><c:roundedCorners val=\"0\"/><c:chart>{title}<c:plotArea><c:layout/>{plot}</c:plotArea>{legend}<c:plotVisOnly val=\"1\"/><c:dispBlanksAs val=\"gap\"/><c:showDLblsOverMax val=\"0\"/></c:chart>{external_data}<c:printSettings><c:headerFooter/><c:pageMargins b=\"0.75\" l=\"0.7\" r=\"0.7\" t=\"0.75\" header=\"0.3\" footer=\"0.3\"/><c:pageSetup/></c:printSettings></c:chartSpace>",
        title = if chart.title.is_empty() {
            "<c:autoTitleDeleted val=\"1\"/>".to_string()
        } else {
            format!(
                "<c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"ja-JP\"/><a:t>{}</a:t></a:r></a:p></c:rich></c:tx><c:layout/><c:overlay val=\"0\"/></c:title><c:autoTitleDeleted val=\"0\"/>",
                escape_xml(&chart.title)
            )
        },
        legend = chart
            .legend
            .as_deref()
            .map(|position| format!(
                "<c:legend><c:legendPos val=\"{}\"/><c:overlay val=\"0\"/></c:legend>",
                escape_xml(position)
            ))
            .unwrap_or_default(),
    ))
}

/// Builds the minimal embedded XLSX that Word opens from “Edit Data”. The
/// workbook deliberately uses inline strings, avoiding a second cache that
/// could drift from ChartML after an update.
pub(crate) fn build_chart_workbook(chart: &RunChart) -> Result<Vec<u8>, String> {
    validate_chart(chart)?;
    let inline_string = |reference: &str, value: &str| {
        format!(
            "<c r=\"{reference}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
            escape_xml(value)
        )
    };
    let mut header = inline_string("A1", "");
    for (index, series) in chart.series.iter().enumerate() {
        header.push_str(&inline_string(
            &format!("{}1", xlsx_column(index + 1)),
            &series.name,
        ));
    }
    let mut rows = format!("<row r=\"1\">{header}</row>");
    for (category_index, category) in chart.categories.iter().enumerate() {
        let row = category_index + 2;
        let mut cells = inline_string(&format!("A{row}"), category);
        for (series_index, series) in chart.series.iter().enumerate() {
            if let Some(value) = series.values[category_index] {
                cells.push_str(&format!(
                    "<c r=\"{}{row}\"><v>{value}</v></c>",
                    xlsx_column(series_index + 1)
                ));
            }
        }
        rows.push_str(&format!("<row r=\"{row}\">{cells}</row>"));
    }
    let sheet_xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><dimension ref=\"A1:{}{}\"/><sheetData>{rows}</sheetData></worksheet>",
        xlsx_column(chart.series.len()),
        chart.categories.len() + 1
    );
    const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>";
    const ROOT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>";
    const WORKBOOK: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>";
    const WORKBOOK_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>";
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (path, content) in [
            ("[Content_Types].xml", CONTENT_TYPES),
            ("_rels/.rels", ROOT_RELS),
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
            ("xl/worksheets/sheet1.xml", sheet_xml.as_str()),
        ] {
            zip.start_file(path, options)
                .map_err(|error| error.to_string())?;
            zip.write_all(content.as_bytes())
                .map_err(|error| error.to_string())?;
        }
        zip.finish().map_err(|error| error.to_string())?;
    }
    Ok(cursor.into_inner())
}

fn chart_palette_rgb(index: usize) -> (u8, u8, u8) {
    const COLORS: [(u8, u8, u8); 8] = [
        (0x4F, 0x46, 0xA5),
        (0x77, 0x7F, 0xC4),
        (0xA4, 0xA9, 0xD8),
        (0x5E, 0x66, 0x8D),
        (0xB8, 0xBB, 0xDA),
        (0x34, 0x3A, 0x5A),
        (0x8B, 0x72, 0xB1),
        (0x67, 0x73, 0xA8),
    ];
    COLORS[index % COLORS.len()]
}

fn set_preview_pixel(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    color: (u8, u8, u8),
) {
    if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
        return;
    }
    let offset = (y as usize * width as usize + x as usize) * 4;
    pixels[offset] = color.0;
    pixels[offset + 1] = color.1;
    pixels[offset + 2] = color.2;
    pixels[offset + 3] = 255;
}

fn draw_preview_line(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    (mut x0, mut y0): (i32, i32),
    (x1, y1): (i32, i32),
    color: (u8, u8, u8),
) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        for offset_y in -1..=1 {
            set_preview_pixel(pixels, width, height, x0, y0 + offset_y, color);
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let twice = 2 * error;
        if twice >= dy {
            error += dy;
            x0 += sx;
        }
        if twice <= dx {
            error += dx;
            y0 += sy;
        }
    }
}

fn fill_preview_rect(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    (left, top): (i32, i32),
    (right, bottom): (i32, i32),
    color: (u8, u8, u8),
) {
    for y in top.max(0)..bottom.min(height as i32) {
        for x in left.max(0)..right.min(width as i32) {
            set_preview_pixel(pixels, width, height, x, y, color);
        }
    }
}

/// Raster preview for the canvas path. Chromium does not implement
/// `createImageBitmap(Blob)` for SVG in workers, so ChartML uses a native PNG
/// preview while the exported document remains a fully editable native chart.
pub(crate) fn chart_preview_data_uri(chart: &RunChart, width: f64, height: f64) -> String {
    let width = width.round().clamp(160.0, 1600.0) as u32;
    let height = height.round().clamp(120.0, 1200.0) as u32;
    let mut pixels = vec![255_u8; width as usize * height as usize * 4];
    let left = (width as f64 * 0.10).max(24.0);
    let right = width as f64 - (width as f64 * 0.04).max(12.0);
    let top = (height as f64 * 0.10).max(16.0);
    let bottom = height as f64 - (height as f64 * 0.12).max(18.0);
    let plot_width = (right - left).max(1.0);
    let plot_height = (bottom - top).max(1.0);

    match chart.kind {
        ChartKind::Bar | ChartKind::Line => {
            let (mut min, mut max) = chart
                .series
                .iter()
                .flat_map(|series| series.values.iter().flatten())
                .fold((0.0_f64, 0.0_f64), |(min, max), value| {
                    (min.min(*value), max.max(*value))
                });
            if (max - min).abs() < f64::EPSILON {
                max += 1.0;
                min = min.min(0.0);
            }
            let y_of = |value: f64| top + (max - value) / (max - min) * plot_height;
            for step in 0..=4 {
                let y = (top + plot_height * step as f64 / 4.0).round() as i32;
                draw_preview_line(
                    &mut pixels,
                    width,
                    height,
                    (left.round() as i32, y),
                    (right.round() as i32, y),
                    (0xD9, 0xDC, 0xE8),
                );
            }
            let category_width = plot_width / chart.categories.len().max(1) as f64;
            if chart.kind == ChartKind::Bar {
                let slot = category_width * 0.78 / chart.series.len().max(1) as f64;
                let baseline = y_of(0.0);
                for (series_index, series) in chart.series.iter().enumerate() {
                    for (category_index, value) in series.values.iter().enumerate() {
                        let Some(value) = value else { continue };
                        let x = left
                            + category_width * category_index as f64
                            + category_width * 0.11
                            + slot * series_index as f64;
                        let y = y_of(*value);
                        fill_preview_rect(
                            &mut pixels,
                            width,
                            height,
                            (x.round() as i32, y.min(baseline).round() as i32),
                            (
                                (x + (slot - 2.0).max(1.0)).round() as i32,
                                y.max(baseline).round() as i32 + 1,
                            ),
                            chart_palette_rgb(series_index),
                        );
                    }
                }
            } else {
                for (series_index, series) in chart.series.iter().enumerate() {
                    let color = chart_palette_rgb(series_index);
                    let mut previous = None;
                    for (category_index, value) in series.values.iter().enumerate() {
                        let Some(value) = value else {
                            previous = None;
                            continue;
                        };
                        let point = (
                            (left + category_width * (category_index as f64 + 0.5)).round() as i32,
                            y_of(*value).round() as i32,
                        );
                        if let Some((x, y)) = previous {
                            draw_preview_line(&mut pixels, width, height, (x, y), point, color);
                        }
                        fill_preview_rect(
                            &mut pixels,
                            width,
                            height,
                            (point.0 - 3, point.1 - 3),
                            (point.0 + 4, point.1 + 4),
                            color,
                        );
                        previous = Some(point);
                    }
                }
            }
        }
        ChartKind::Pie => {
            let values = &chart.series[0].values;
            let total = values
                .iter()
                .flatten()
                .filter(|value| **value > 0.0)
                .sum::<f64>();
            if total > 0.0 {
                let center_x = (left + right) / 2.0;
                let center_y = (top + bottom) / 2.0;
                let radius = plot_width.min(plot_height) * 0.42;
                let cumulative = values
                    .iter()
                    .map(|value| value.filter(|value| *value > 0.0).unwrap_or(0.0) / total)
                    .scan(0.0, |sum, fraction| {
                        *sum += fraction;
                        Some(*sum)
                    })
                    .collect::<Vec<_>>();
                for y in (center_y - radius).floor() as i32..=(center_y + radius).ceil() as i32 {
                    for x in (center_x - radius).floor() as i32..=(center_x + radius).ceil() as i32
                    {
                        let dx = x as f64 - center_x;
                        let dy = y as f64 - center_y;
                        if dx * dx + dy * dy > radius * radius {
                            continue;
                        }
                        let fraction = (dy.atan2(dx) + std::f64::consts::FRAC_PI_2)
                            .rem_euclid(std::f64::consts::TAU)
                            / std::f64::consts::TAU;
                        let index = cumulative
                            .iter()
                            .position(|limit| fraction <= *limit)
                            .unwrap_or_else(|| cumulative.len().saturating_sub(1));
                        set_preview_pixel(
                            &mut pixels,
                            width,
                            height,
                            x,
                            y,
                            chart_palette_rgb(index),
                        );
                    }
                }
            }
        }
    }

    let mut png_bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("write chart PNG header");
        writer
            .write_image_data(&pixels)
            .expect("write chart PNG pixels");
    }
    format!("data:image/png;base64,{}", BASE64.encode(png_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plot_xml(plot: &str, values: &str) -> String {
        format!(
            r#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:{plot}><c:ser><c:tx><c:v>S</c:v></c:tx>{values}</c:ser></c:{plot}></c:plotArea></c:chart></c:chartSpace>"#
        )
    }

    #[test]
    fn preview_parse_maps_unauthorable_kinds_without_loosening_editing() {
        let category_values = r#"<c:cat><c:strLit><c:ptCount val="2"/><c:pt idx="0"><c:v>A</c:v></c:pt><c:pt idx="1"><c:v>B</c:v></c:pt></c:strLit></c:cat><c:val><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>3</c:v></c:pt><c:pt idx="1"><c:v>5</c:v></c:pt></c:numLit></c:val>"#;
        for (plot, kind) in [
            ("doughnutChart", ChartKind::Pie),
            ("pie3DChart", ChartKind::Pie),
            ("bar3DChart", ChartKind::Bar),
            ("areaChart", ChartKind::Line),
        ] {
            let xml = plot_xml(plot, category_values);
            assert!(parse_chart_xml(&xml).is_err(), "{plot} stays non-editable");
            let preview = parse_chart_xml_for_preview(&xml).expect(plot);
            assert_eq!(preview.kind, kind, "{plot}");
            assert_eq!(preview.series[0].values, vec![Some(3.0), Some(5.0)]);
        }
        let scatter = plot_xml(
            "scatterChart",
            r#"<c:xVal><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>1</c:v></c:pt><c:pt idx="1"><c:v>2</c:v></c:pt></c:numLit></c:xVal><c:yVal><c:numLit><c:ptCount val="2"/><c:pt idx="0"><c:v>4</c:v></c:pt><c:pt idx="1"><c:v>8</c:v></c:pt></c:numLit></c:yVal>"#,
        );
        let preview = parse_chart_xml_for_preview(&scatter).expect("scatter");
        assert_eq!(preview.categories, vec!["1".to_string(), "2".to_string()]);
        assert_eq!(preview.series[0].values, vec![Some(4.0), Some(8.0)]);
        assert!(parse_chart_xml_for_preview(&plot_xml("bubbleChart", "")).is_err());
    }

    #[test]
    fn authored_chart_round_trips_xml_reserved_text_and_gaps() {
        let chart = RunChart {
            kind: ChartKind::Line,
            title: "売上 & <利益>".into(),
            categories: vec!["1月".into(), "2&3月".into()],
            series: vec![ChartSeries {
                name: "東 <西>".into(),
                values: vec![Some(12.5), None],
            }],
            ..Default::default()
        };
        let xml = build_chart_xml(&chart, Some("rId1")).expect("chart XML");
        let parsed = parse_chart_xml(&xml).expect("parse chart XML");
        assert_eq!(parsed, chart);
        let data_uri = chart_preview_data_uri(&chart, 400.0, 240.0);
        let encoded = data_uri
            .strip_prefix("data:image/png;base64,")
            .expect("PNG data URI");
        let png = BASE64.decode(encoded).expect("decode PNG");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }
}
