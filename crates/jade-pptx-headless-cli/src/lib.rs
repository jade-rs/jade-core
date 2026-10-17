//! Reading a deck from a shell script or an agent.
//!
//! These are the functions behind the CLI's commands, separated so tests and
//! future callers (an MCP server) run them without spawning a process.
//!
//! Two families: queries over what the deck says (`summarize`, `view`,
//! `text`, `find`), and queries over what it would draw (`layout_view`,
//! `check`). The latter run the real layout engine with the deterministic
//! heuristic measurer, so they answer "does this fit" the way the renderer
//! would, without a font or a canvas.

use std::collections::BTreeMap;

use jade_pptx_headless_api::{
    ChartSeriesView, ChartView, DeckSummary, IssueKind, LayoutIssue, OutlineView, PlacedShapeView,
    RectPx, ShapeKind, ShapeView, SlideLayoutView, SlideSummary, SlideView, TextMatch,
};
use jade_pptx_layout::{layout_deck_slide, HeuristicMeasure, PlacedGeometry, PlacedOrigin};
use jade_pptx_reader::{read_deck, Deck, Placeholder, ShapeItem, Table, TextBody};

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

/// The deck at a glance: size, slide order, titles, shape counts.
pub fn deck_summary(bytes: &[u8]) -> Result<DeckSummary> {
    Ok(summarize(&read_deck(bytes)?))
}

/// [`deck_summary`] over an already-parsed deck, for callers holding one.
pub fn summarize(deck: &Deck) -> DeckSummary {
    let slides = deck
        .slides
        .iter()
        .enumerate()
        .map(|(index, slide)| SlideSummary {
            index: index + 1,
            part_name: slide.part_name.clone(),
            title: slide_title(&slide.content.shapes),
            shape_count: slide.content.shapes.len(),
            has_notes: slide.notes.is_some(),
            hidden: slide.content.hidden,
        })
        .collect();
    DeckSummary {
        slide_count: deck.slides.len(),
        width_px: deck.slide_size.width_px(),
        height_px: deck.slide_size.height_px(),
        slides,
    }
}

/// One slide's shapes with their text. `index` is 1-based.
pub fn slide_view(bytes: &[u8], index: usize) -> Result<SlideView> {
    view(&read_deck(bytes)?, index)
}

/// [`slide_view`] over an already-parsed deck.
pub fn view(deck: &Deck, index: usize) -> Result<SlideView> {
    let slide = checked_slide(deck, index)?;
    // Placed geometry keyed by cNvPr id: where each shape actually lands,
    // placeholders included, and a table as the union of its cells.
    let mut rects: BTreeMap<u32, RectPx> = BTreeMap::new();
    if let Some(layout) = layout_deck_slide(deck, index - 1, &HeuristicMeasure) {
        for placed in layout
            .shapes
            .iter()
            .filter(|placed| placed.origin == PlacedOrigin::Slide)
        {
            let Some(id) = placed.source_id else { continue };
            let rect = rect_px(&placed.rect);
            rects
                .entry(id)
                .and_modify(|existing| *existing = union(*existing, rect))
                .or_insert(rect);
        }
    }
    Ok(SlideView {
        index,
        shapes: slide
            .content
            .shapes
            .iter()
            .map(|item| shape_view(item, &rects))
            .collect(),
        notes: slide.notes.clone(),
    })
}

/// Every slide's text, in order, as readable plain text. Table rows come
/// out tab-separated; speaker notes follow the slide, quoted with `> `.
pub fn deck_text(bytes: &[u8]) -> Result<String> {
    Ok(text(&read_deck(bytes)?))
}

/// [`deck_text`] over an already-parsed deck.
pub fn text(deck: &Deck) -> String {
    let blocks: Vec<String> = deck
        .slides
        .iter()
        .enumerate()
        .map(|(index, slide)| {
            let mut block = format!("## Slide {}\n", index + 1);
            collect_readable(&slide.content.shapes, &mut block);
            if let Some(notes) = &slide.notes {
                for line in notes.lines() {
                    block.push_str("> ");
                    block.push_str(line);
                    block.push('\n');
                }
            }
            block
        })
        .collect();
    blocks.join("\n")
}

/// Shapes whose text contains `needle`. Case-insensitive unless asked.
pub fn find_text(bytes: &[u8], needle: &str, case_sensitive: bool) -> Result<Vec<TextMatch>> {
    Ok(find(&read_deck(bytes)?, needle, case_sensitive))
}

/// [`find_text`] over an already-parsed deck. Table cells and speaker notes
/// are searched too.
pub fn find(deck: &Deck, needle: &str, case_sensitive: bool) -> Vec<TextMatch> {
    let folded_needle = if case_sensitive {
        needle.to_string()
    } else {
        needle.to_lowercase()
    };
    let contains = |text: &str| {
        let haystack = if case_sensitive {
            text.to_string()
        } else {
            text.to_lowercase()
        };
        haystack.contains(&folded_needle)
    };
    let mut matches = Vec::new();
    for (index, slide) in deck.slides.iter().enumerate() {
        collect_text(&slide.content.shapes, &mut |name, text, _| {
            if contains(text) {
                matches.push(TextMatch {
                    slide: index + 1,
                    shape: name.to_string(),
                    text: text.to_string(),
                    in_notes: false,
                });
            }
        });
        if let Some(notes) = &slide.notes {
            if contains(notes) {
                matches.push(TextMatch {
                    slide: index + 1,
                    shape: "Notes".to_string(),
                    text: notes.clone(),
                    in_notes: true,
                });
            }
        }
    }
    matches
}

/// What the layout engine would draw for slide `index` (1-based).
pub fn slide_layout(bytes: &[u8], index: usize) -> Result<SlideLayoutView> {
    layout_view(&read_deck(bytes)?, index)
}

/// [`slide_layout`] over an already-parsed deck.
pub fn layout_view(deck: &Deck, index: usize) -> Result<SlideLayoutView> {
    checked_slide(deck, index)?;
    let layout = layout_deck_slide(deck, index - 1, &HeuristicMeasure)
        .ok_or_else(|| format!("slide {index} did not lay out"))?;
    Ok(SlideLayoutView {
        index,
        width_px: layout.width_px,
        height_px: layout.height_px,
        background: layout.background.clone(),
        shapes: layout
            .shapes
            .iter()
            .map(|placed| PlacedShapeView {
                name: placed.name.clone(),
                id: placed.source_id,
                origin: match placed.origin {
                    PlacedOrigin::Slide => "slide",
                    PlacedOrigin::Layout => "layout",
                    PlacedOrigin::Master => "master",
                }
                .to_string(),
                rect: rect_px(&placed.rect),
                text_region: placed.text_region.as_ref().map(rect_px),
                rotation_deg: placed.paint_transform().rotation_rad.to_degrees(),
                geometry: match &placed.geometry {
                    PlacedGeometry::Rect => "rect",
                    PlacedGeometry::RoundRect { .. } => "round_rect",
                    PlacedGeometry::Ellipse => "ellipse",
                    PlacedGeometry::Line => "line",
                    PlacedGeometry::Polygon(_) => "polygon",
                    PlacedGeometry::Path(_) => "path",
                }
                .to_string(),
                fill: placed.fill.clone(),
                outline: placed.outline.as_ref().map(|outline| OutlineView {
                    color: outline.color.clone(),
                    width_px: outline.width_px,
                    dashed: !outline.dash_px.is_empty(),
                }),
                lines: placed
                    .lines
                    .iter()
                    .map(|line| {
                        line.runs
                            .iter()
                            .map(|run| run.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                            .trim_end()
                            .to_string()
                    })
                    .collect(),
                text_overflow_px: placed.text_overflow_px(),
            })
            .collect(),
    })
}

/// Layout problems a presenter would notice, on one slide (1-based) or the
/// whole deck.
pub fn check_layout(bytes: &[u8], index: Option<usize>) -> Result<Vec<LayoutIssue>> {
    check(&read_deck(bytes)?, index)
}

/// [`check_layout`] over an already-parsed deck.
pub fn check(deck: &Deck, index: Option<usize>) -> Result<Vec<LayoutIssue>> {
    let range = match index {
        Some(index) => {
            checked_slide(deck, index)?;
            index..index + 1
        }
        None => 1..deck.slides.len() + 1,
    };
    let mut issues = Vec::new();
    for slide_index in range {
        let Some(layout) = layout_deck_slide(deck, slide_index - 1, &HeuristicMeasure) else {
            continue;
        };
        for placed in layout
            .shapes
            .iter()
            .filter(|placed| placed.origin == PlacedOrigin::Slide)
        {
            // A cell border or connector has no area to be inside or outside.
            let has_area = placed.rect.width > 0.0 && placed.rect.height > 0.0;
            // The heuristic line box is a fifth taller than most glyphs, so
            // an overflow smaller than that on the last line is the line box,
            // not the text: PowerPoint shows such a shape looking fine.
            let overflow = placed.text_overflow_px();
            let region_label = if placed.text_region.is_some() {
                "geometry text region"
            } else {
                "shape"
            };
            let last_line_height = placed
                .lines
                .iter()
                .rev()
                .find(|line| line.runs.iter().any(|run| !run.text.trim().is_empty()))
                .map(|line| line.height)
                .unwrap_or(0.0);
            if overflow > (0.2 * last_line_height).max(0.5) {
                issues.push(LayoutIssue {
                    slide: slide_index,
                    shape: placed.name.clone(),
                    id: placed.source_id,
                    kind: IssueKind::TextOverflow,
                    detail: format!(
                        "text runs {overflow:.1} px past the bottom of the {region_label} ({:.0} px tall)",
                        placed.text_region.map_or(placed.rect.height, |r| r.height)
                    ),
                    amount_px: overflow,
                });
            }
            let too_wide = placed.text_overflow_right_px();
            if too_wide > 0.5 {
                issues.push(LayoutIssue {
                    slide: slide_index,
                    shape: placed.name.clone(),
                    id: placed.source_id,
                    kind: IssueKind::TextTooWide,
                    detail: format!(
                        "text runs {too_wide:.1} px past the right edge of the {region_label} ({:.0} px wide)",
                        placed.text_region.map_or(placed.rect.width, |r| r.width)
                    ),
                    amount_px: too_wide,
                });
            }
            if has_area {
                let right = placed.rect.x + placed.rect.width;
                let bottom = placed.rect.y + placed.rect.height;
                let outside = [
                    -right,
                    -bottom,
                    placed.rect.x - layout.width_px,
                    placed.rect.y - layout.height_px,
                ]
                .into_iter()
                .fold(f64::NEG_INFINITY, f64::max);
                if outside >= 0.0 {
                    issues.push(LayoutIssue {
                        slide: slide_index,
                        shape: placed.name.clone(),
                        id: placed.source_id,
                        kind: IssueKind::OutsideSlide,
                        detail: format!(
                            "the shape lies {outside:.1} px outside the {:.0}×{:.0} slide",
                            layout.width_px, layout.height_px
                        ),
                        amount_px: outside,
                    });
                }
            }
        }
    }
    Ok(issues)
}

/// The bytes of a media part (a picture's `image_part`), with the MIME type
/// its extension implies.
pub fn media(bytes: &[u8], part: &str) -> Result<(Vec<u8>, String)> {
    let package = jade_opc::OpcPackage::open(bytes)?;
    let part_name = part.trim_start_matches('/');
    let data = package
        .part(part_name)
        .ok_or_else(|| format!("no part named {part_name} in the package"))?
        .to_vec();
    Ok((data, media_type(part_name).to_string()))
}

fn media_type(part: &str) -> &'static str {
    match part.rsplit('.').next().map(|ext| ext.to_ascii_lowercase()) {
        Some(ext) if ext == "png" => "image/png",
        Some(ext) if ext == "jpg" || ext == "jpeg" => "image/jpeg",
        Some(ext) if ext == "gif" => "image/gif",
        Some(ext) if ext == "svg" => "image/svg+xml",
        Some(ext) if ext == "emf" => "image/emf",
        Some(ext) if ext == "wmf" => "image/wmf",
        Some(ext) if ext == "tif" || ext == "tiff" => "image/tiff",
        Some(ext) if ext == "bmp" => "image/bmp",
        Some(ext) if ext == "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

fn rect_px(rect: &jade_pptx_layout::Rect) -> RectPx {
    RectPx {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

fn union(a: RectPx, b: RectPx) -> RectPx {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    RectPx {
        x,
        y,
        width: (a.x + a.width).max(b.x + b.width) - x,
        height: (a.y + a.height).max(b.y + b.height) - y,
    }
}

fn checked_slide(deck: &Deck, index: usize) -> Result<&jade_pptx_reader::DeckSlide> {
    index
        .checked_sub(1)
        .and_then(|zero_based| deck.slides.get(zero_based))
        .ok_or_else(|| {
            format!(
                "slide {index} is out of range: the deck has {} slide(s), numbered from 1",
                deck.slides.len()
            )
            .into()
        })
}

fn body_text(body: &TextBody) -> Option<String> {
    let text = raw_body_text(body);
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn raw_body_text(body: &TextBody) -> String {
    body.paragraphs
        .iter()
        .map(|paragraph| {
            paragraph
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn body_hyperlinks(body: &TextBody) -> Vec<String> {
    let mut links = Vec::new();
    for paragraph in &body.paragraphs {
        for run in &paragraph.runs {
            if let Some(target) = run.hyperlink.as_ref().and_then(|link| link.target.as_ref()) {
                if !links.contains(target) {
                    links.push(target.clone());
                }
            }
        }
    }
    links
}

fn table_rows(table: &Table) -> Vec<Vec<String>> {
    table
        .rows
        .iter()
        .map(|row| {
            row.cells
                .iter()
                .map(|cell| {
                    if cell.h_merge || cell.v_merge {
                        String::new()
                    } else {
                        cell.text.as_ref().map(raw_body_text).unwrap_or_default()
                    }
                })
                .collect()
        })
        .collect()
}

fn placeholder_label(placeholder: &Placeholder) -> String {
    match placeholder.index {
        Some(index) => format!("{}#{index}", placeholder.kind),
        None => placeholder.kind.clone(),
    }
}

/// The text of a slide's title placeholder, when it has one.
///
/// Public because a filmstrip wants one slide's label without summarising the
/// whole deck, and "what counts as the title" should not be answered twice.
pub fn slide_title(shapes: &[ShapeItem]) -> Option<String> {
    shapes.iter().find_map(|item| match item {
        ShapeItem::Shape(shape) => {
            let kind = shape.placeholder.as_ref()?.kind.as_str();
            if kind == "title" || kind == "ctrTitle" {
                shape.text.as_ref().and_then(body_text)
            } else {
                None
            }
        }
        _ => None,
    })
}

fn shape_view(item: &ShapeItem, rects: &BTreeMap<u32, RectPx>) -> ShapeView {
    let rect_of = |id: Option<u32>| id.and_then(|id| rects.get(&id).copied());
    match item {
        ShapeItem::Shape(shape) => ShapeView {
            kind: ShapeKind::Shape,
            id: shape.id,
            name: shape.name.clone(),
            placeholder: shape.placeholder.as_ref().map(placeholder_label),
            geometry: shape.preset_geometry.clone(),
            rect: rect_of(shape.id),
            text: shape.text.as_ref().and_then(body_text),
            rows: Vec::new(),
            image_part: None,
            chart: None,
            hyperlinks: shape.text.as_ref().map(body_hyperlinks).unwrap_or_default(),
            children: Vec::new(),
        },
        ShapeItem::Picture(picture) => ShapeView {
            kind: ShapeKind::Picture,
            id: picture.id,
            name: picture.name.clone(),
            placeholder: None,
            geometry: None,
            rect: rect_of(picture.id),
            text: None,
            rows: Vec::new(),
            image_part: picture.image_part.clone(),
            chart: None,
            hyperlinks: Vec::new(),
            children: Vec::new(),
        },
        ShapeItem::Group(group) => {
            let children: Vec<ShapeView> = group
                .children
                .iter()
                .map(|child| shape_view(child, rects))
                .collect();
            // A group's footprint is its children's.
            let rect = children.iter().filter_map(|child| child.rect).reduce(union);
            ShapeView {
                kind: ShapeKind::Group,
                id: group.id,
                name: group.name.clone(),
                placeholder: None,
                geometry: None,
                rect,
                text: None,
                rows: Vec::new(),
                image_part: None,
                chart: None,
                hyperlinks: Vec::new(),
                children,
            }
        }
        ShapeItem::Chart(chart) => ShapeView {
            kind: ShapeKind::Chart,
            id: chart.id,
            name: chart.name.clone(),
            placeholder: None,
            geometry: None,
            rect: rect_of(chart.id),
            text: chart.title.clone().filter(|title| !title.trim().is_empty()),
            rows: Vec::new(),
            image_part: None,
            chart: Some(chart_view(chart)),
            hyperlinks: Vec::new(),
            children: Vec::new(),
        },
        ShapeItem::Table(table) => ShapeView {
            kind: ShapeKind::Table,
            id: table.id,
            name: table.name.clone(),
            placeholder: None,
            geometry: None,
            rect: rect_of(table.id),
            text: None,
            rows: table_rows(table),
            image_part: None,
            chart: None,
            hyperlinks: table
                .rows
                .iter()
                .flat_map(|row| row.cells.iter())
                .filter_map(|cell| cell.text.as_ref())
                .flat_map(body_hyperlinks)
                .fold(Vec::new(), |mut links, link| {
                    if !links.contains(&link) {
                        links.push(link);
                    }
                    links
                }),
            children: Vec::new(),
        },
    }
}

fn chart_view(chart: &jade_pptx_reader::Chart) -> ChartView {
    ChartView {
        kind: chart.kind.clone(),
        title: chart.title.clone(),
        series: chart
            .series
            .iter()
            .map(|series| ChartSeriesView {
                name: series.name.clone(),
                categories: series.categories.clone(),
                values: series.values.clone(),
            })
            .collect(),
    }
}

/// A chart as readable text: its title, then one line per series with
/// `category=value` pairs.
fn chart_text(chart: &jade_pptx_reader::Chart) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(title) = chart
        .title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
    {
        lines.push(title.to_string());
    }
    for series in &chart.series {
        let points: Vec<String> = series
            .values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let category = series
                    .categories
                    .get(index)
                    .cloned()
                    .unwrap_or_else(|| (index + 1).to_string());
                match value {
                    Some(value) => format!("{category}={value}"),
                    None => format!("{category}="),
                }
            })
            .collect();
        let name = series.name.clone().unwrap_or_else(|| "series".to_string());
        lines.push(format!("{name}: {}", points.join(", ")));
    }
    lines.join("\n")
}

/// Spreadsheet-style column letters: 0 → A, 25 → Z, 26 → AA.
fn column_letters(index: usize) -> String {
    let mut letters = Vec::new();
    let mut n = index + 1;
    while n > 0 {
        let rem = (n - 1) % 26;
        letters.push((b'A' + rem as u8) as char);
        n = (n - 1) / 26;
    }
    letters.iter().rev().collect()
}

/// Calls `visit(shape_name, text, kind)` for every shape with text,
/// depth-first. Table rows visit as one tab-separated line each, and every
/// cell again by its `Table!B2` name so a find can point at it.
fn collect_text(shapes: &[ShapeItem], visit: &mut impl FnMut(&str, &str, ShapeKind)) {
    for item in shapes {
        match item {
            ShapeItem::Shape(shape) => {
                if let Some(text) = shape.text.as_ref().and_then(body_text) {
                    visit(&shape.name, &text, ShapeKind::Shape);
                }
            }
            ShapeItem::Group(group) => collect_text(&group.children, visit),
            ShapeItem::Table(table) => {
                for (row_index, row) in table.rows.iter().enumerate() {
                    let mut column = 0usize;
                    for cell in &row.cells {
                        if !(cell.h_merge || cell.v_merge) {
                            if let Some(text) = cell.text.as_ref().and_then(body_text) {
                                let name = format!(
                                    "{}!{}{}",
                                    table.name,
                                    column_letters(column),
                                    row_index + 1
                                );
                                visit(&name, &text, ShapeKind::Table);
                            }
                        }
                        column += cell.grid_span.max(1) as usize;
                    }
                }
            }
            ShapeItem::Chart(chart) => {
                let text = chart_text(chart);
                if !text.trim().is_empty() {
                    visit(&chart.name, &text, ShapeKind::Chart);
                }
            }
            ShapeItem::Picture(_) => {}
        }
    }
}

/// Appends every shape's text as a reader would want it: one line per
/// paragraph, table rows tab-separated, groups flattened.
fn collect_readable(shapes: &[ShapeItem], out: &mut String) {
    for item in shapes {
        match item {
            ShapeItem::Shape(shape) => {
                if let Some(text) = shape.text.as_ref().and_then(body_text) {
                    out.push_str(&text);
                    out.push('\n');
                }
            }
            ShapeItem::Group(group) => collect_readable(&group.children, out),
            ShapeItem::Table(table) => {
                for row in table_rows(table) {
                    out.push_str(&row.join("\t"));
                    out.push('\n');
                }
            }
            ShapeItem::Chart(chart) => {
                let text = chart_text(chart);
                if !text.trim().is_empty() {
                    out.push_str(&text);
                    out.push('\n');
                }
            }
            ShapeItem::Picture(_) => {}
        }
    }
}
