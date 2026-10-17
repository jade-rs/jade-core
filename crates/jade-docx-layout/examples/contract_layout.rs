use fontdb::{Database, Family, Query, Style as FontStyle, Weight};
use fontdue::Font;
use futures::executor::block_on;
use jade_docx_layout::font_resolver::{resolve_run_style_for_wrap, ResolvedRunStyle};
use jade_docx_layout::layout_cache::{TextMeasureCache, TextMeasurer};
use jade_docx_layout::line_metrics::{resolve_effective_snap_to_grid, resolve_line_height_points};
use jade_docx_layout::line_wrap::{
    apply_paragraph_indent, merge_east_asian_rules, resolve_tab_stops, wrap_text_into_lines,
    EastAsianLineBreakingRules, LineWrapContext, ParagraphStyle, RunStyle,
};
use jade_docx_layout::units::points_to_px;
use jade_docx_reader::{
    BodyElement, DocGrid, Document, DocumentParser, OpenXmlPackage, RelationshipTypes, Run,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::rc::Rc;

struct SystemTextMeasurer {
    db: Database,
    font_cache: RefCell<HashMap<String, Font>>,
    default_font_size_pt: f32,
}

impl SystemTextMeasurer {
    fn new(default_font_size_pt: f32) -> Result<Self, Box<dyn Error>> {
        let mut db = Database::new();
        db.load_system_fonts();
        Ok(Self {
            db,
            font_cache: RefCell::new(HashMap::new()),
            default_font_size_pt,
        })
    }

    fn parse_descriptor(&self, descriptor: &str) -> (Vec<String>, Weight, FontStyle, f32) {
        let mut size_pt = self.default_font_size_pt;
        let mut weight = Weight::NORMAL;
        let mut style = FontStyle::Normal;
        let families: String;

        let tokens: Vec<&str> = descriptor.split_whitespace().collect();
        let mut size_idx = None;
        for (idx, token) in tokens.iter().enumerate() {
            if let Some(value) = token.strip_suffix("pt") {
                if let Ok(size) = value.parse::<f32>() {
                    size_pt = size;
                    size_idx = Some(idx);
                    break;
                }
            }
        }

        if let Some(idx) = size_idx {
            let prefix = tokens[..idx].join(" ");
            let suffix = tokens[idx + 1..].join(" ");
            families = suffix;
            let prefix_lower = prefix.to_ascii_lowercase();
            if prefix_lower.contains("bold") {
                weight = Weight::BOLD;
            }
            if prefix_lower.contains("italic") {
                style = FontStyle::Italic;
            }
        } else {
            families = tokens.join(" ");
        }

        let family_list = families
            .split(',')
            .map(|item| item.trim().trim_matches('"').trim_matches('\''))
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string())
            .collect::<Vec<_>>();

        (family_list, weight, style, size_pt)
    }

    fn resolve_font(&self, descriptor: &str) -> Result<Font, Box<dyn Error>> {
        if let Some(font) = self.font_cache.borrow().get(descriptor) {
            return Ok(font.clone());
        }

        let (families, weight, style, _size_pt) = self.parse_descriptor(descriptor);
        let mut query_families: Vec<Family<'_>> = Vec::new();
        for name in &families {
            query_families.push(Family::Name(name));
        }
        if query_families.is_empty() {
            query_families.push(Family::Serif);
        }

        let query = Query {
            families: &query_families,
            weight,
            style,
            ..Default::default()
        };
        let fallback = Query {
            families: &[Family::Serif],
            weight: Weight::NORMAL,
            style: FontStyle::Normal,
            ..Default::default()
        };

        let id = self
            .db
            .query(&query)
            .or_else(|| self.db.query(&fallback))
            .ok_or("no system fonts available")?;

        let font = self
            .db
            .with_face_data(id, |data, _| {
                Font::from_bytes(data, fontdue::FontSettings::default())
            })
            .ok_or("failed to load font data")??;

        self.font_cache
            .borrow_mut()
            .insert(descriptor.to_string(), font.clone());

        Ok(font)
    }
}

impl TextMeasurer for SystemTextMeasurer {
    type Error = Box<dyn Error>;

    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
        let font = self.resolve_font(font_descriptor)?;
        let (_, _, _, size_pt) = self.parse_descriptor(font_descriptor);
        let px_size = points_to_px(size_pt);
        let mut width = 0.0_f64;
        for ch in text.chars() {
            let metrics = font.metrics(ch, px_size as f32);
            width += metrics.advance_width as f64;
        }
        Ok(width)
    }
}

struct ExampleLayoutContext {
    measurer: SystemTextMeasurer,
    cache: RefCell<TextMeasureCache>,
    doc_grid_char_spacing_px: Option<f64>,
}

impl LineWrapContext for ExampleLayoutContext {
    type Error = Box<dyn Error>;

    fn resolve_run_style(
        &self,
        properties: Option<&jade_docx_reader::RunProperties>,
    ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
        Ok(resolve_run_style_for_wrap(
            properties,
            self.doc_grid_char_spacing_px,
        ))
    }

    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
        let mut cache = self.cache.borrow_mut();
        cache.measure_with(&self.measurer, text, font_descriptor)
    }
}

fn load_document(path: &str) -> Result<Document, Box<dyn Error>> {
    let data = fs::read(path)?;
    let package = block_on(OpenXmlPackage::load(&data))?;
    let doc_target = package
        .relationships
        .iter()
        .find(|rel| rel.r#type == RelationshipTypes::OfficeDocument.as_str())
        .ok_or("document.xml relationship not found")?
        .target
        .clone();
    let xml = package.get(&doc_target).ok_or("document.xml not found")?;
    let xml = String::from_utf8_lossy(xml);
    let parser = DocumentParser::new();
    parser.parse_document_xml(&xml)
}

fn first_paragraph(document: &Document) -> Option<&jade_docx_reader::Paragraph> {
    let body = document.body.as_ref()?;
    body.elements.iter().find_map(BodyElement::as_paragraph)
}

fn build_run_styles(paragraph: &jade_docx_reader::Paragraph) -> (String, Vec<RunStyle>) {
    let mut text = String::new();
    let mut run_styles = Vec::new();

    for run in &paragraph.runs {
        let Run { text: run_text, .. } = run;
        if run_text.is_empty() {
            continue;
        }
        let start = text.len();
        text.push_str(run_text);
        let end = text.len();
        run_styles.push(RunStyle {
            start,
            end,
            properties: run.properties.clone().map(Rc::new),
            revision: run.revision.clone(),
            hyperlink: run.hyperlink.clone(),
            inline_object: None,
        });
    }

    (text, run_styles)
}

fn estimate_paragraph_font_size(paragraph: &jade_docx_reader::Paragraph) -> f32 {
    let mut max_size = 12.0_f32;
    for run in &paragraph.runs {
        if let Some(props) = &run.properties {
            if let Some(size) = props.font_size {
                max_size = max_size.max(size);
            }
        }
    }
    max_size
}

fn resolve_doc_grid_char_spacing(grid: Option<&DocGrid>) -> Option<f64> {
    let grid = grid?;
    let matches_lines_and_chars = grid
        .grid_type
        .as_deref()
        .map(|kind| kind.eq_ignore_ascii_case("linesAndChars"))
        .unwrap_or(false);
    if matches_lines_and_chars {
        grid.char_space.map(points_to_px)
    } else {
        None
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let doc_path = "public/fixtures/contract.docx";
    let document = load_document(doc_path)?;
    let paragraph = first_paragraph(&document).ok_or("no paragraph found")?;
    let (text, run_styles) = build_run_styles(paragraph);
    if text.trim().is_empty() {
        return Err("first paragraph has no text".into());
    }

    let page_settings = document.page_settings.as_ref();
    let page_width_pt = page_settings.and_then(|s| s.width).unwrap_or(595.0);
    let page_height_pt = page_settings.and_then(|s| s.height).unwrap_or(842.0);
    let margin_left_pt = page_settings.and_then(|s| s.margin_left).unwrap_or(72.0);
    let margin_right_pt = page_settings.and_then(|s| s.margin_right).unwrap_or(72.0);
    let margin_top_pt = page_settings.and_then(|s| s.margin_top).unwrap_or(72.0);
    let margin_bottom_pt = page_settings.and_then(|s| s.margin_bottom).unwrap_or(72.0);
    let section_content_height_pt = (page_height_pt - margin_top_pt - margin_bottom_pt).max(0.0);

    let mut base_x = points_to_px(margin_left_pt);
    let column_right_edge = points_to_px(page_width_pt - margin_right_pt);

    let mut paragraph_style = ParagraphStyle::default();
    if let Some(props) = paragraph.properties.as_ref() {
        if let Some(alignment) = &props.alignment {
            paragraph_style.alignment = alignment.to_string();
        }
        if let Some(indent) = &props.indent {
            apply_paragraph_indent(
                &mut base_x,
                &mut paragraph_style,
                indent,
                props.numbering.is_some(),
            );
        }
        paragraph_style.tab_stops = resolve_tab_stops(&props.tabs);
        let defaults =
            EastAsianLineBreakingRules::from_settings(&document.east_asian_line_breaking);
        paragraph_style.east_asian_rules =
            merge_east_asian_rules(&defaults, props.east_asian_settings.as_ref());
        paragraph_style.enable_line_break_rules =
            paragraph_style.east_asian_rules.use_east_asian_layout;
    }

    let doc_grid = document
        .default_section
        .as_ref()
        .and_then(|section| section.doc_grid.as_ref());
    let doc_grid_char_spacing_px = resolve_doc_grid_char_spacing(doc_grid);

    let ctx = ExampleLayoutContext {
        measurer: SystemTextMeasurer::new(estimate_paragraph_font_size(paragraph))?,
        cache: RefCell::new(TextMeasureCache::default()),
        doc_grid_char_spacing_px,
    };

    let snap_to_grid = resolve_effective_snap_to_grid(
        paragraph
            .properties
            .as_ref()
            .and_then(|props| props.snap_to_grid),
        document.snap_to_grid_default,
        doc_grid,
    );
    let line_height_points = resolve_line_height_points(
        estimate_paragraph_font_size(paragraph),
        paragraph
            .properties
            .as_ref()
            .and_then(|props| props.spacing.as_ref()),
        doc_grid,
        snap_to_grid,
        Some(section_content_height_pt),
        None,
    );
    let spacing_before = paragraph
        .properties
        .as_ref()
        .and_then(|props| props.spacing.as_ref())
        .and_then(|spacing| {
            if let Some(before) = spacing.before {
                Some(points_to_px(before))
            } else {
                spacing
                    .before_lines
                    .map(|lines| points_to_px(line_height_points * lines))
            }
        })
        .unwrap_or(0.0);

    let start_y = points_to_px(margin_top_pt) + spacing_before;

    let lines = wrap_text_into_lines(
        &ctx,
        &text,
        &run_styles,
        base_x,
        &paragraph_style,
        0,
        column_right_edge,
    )?;
    let first = lines.first().ok_or("no wrapped lines")?;
    let line_text: String = first
        .segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect();
    let line_x = base_x + paragraph_style.first_line_indent;

    println!("line: {}", line_text.trim_end());
    println!("x: {:.2}, y: {:.2}", line_x, start_y);
    Ok(())
}
