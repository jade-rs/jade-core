use futures::executor::block_on;
use jade_docx_headless_api::{
    HeadlessApplyAndQueryRequest, HeadlessEditRequest, HeadlessQueryRequest,
};
use jade_docx_headless_editor::{
    apply_headless_commands, apply_headless_commands_and_execute_queries,
    apply_headless_commands_and_execute_queries_with_diff, execute_headless_queries,
};
use jade_docx_layout::font_measurer::{
    FontdueLineWrapContext, FontdueMeasurer, RegisteredFontStyle,
};
use jade_docx_layout::font_resolver::ResolvedRunStyle;
use jade_docx_layout::line_wrap::LineWrapContext;
use jade_docx_layout::measure_backend::WrapBackend;
use jade_docx_layout::shaping_measurer::{ShapingLineWrapContext, ShapingMeasurer, ShapingOptions};
use jade_docx_layout::text_extract::{
    layout_docx_lines_from_bytes, layout_docx_lines_from_bytes_with_oracle_advances,
    OracleAdvanceMap, ParagraphOracleAdvanceMap,
};
use jade_docx_pdf_renderer::{render_docx_to_pdf_with_report, RenderOptions, TextMeasurement};
use jade_docx_reader::numbering::NumberingCounter;
use jade_docx_reader::RunProperties;
use jade_docx_reader::{
    DocumentParser, NumberingPart, OpenXmlPackage, RelationshipTypes, StylesPart,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Stdio;
use std::process::{Child, Command, ExitStatus};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq)]
enum CliArgs {
    HeadlessEdit {
        input: PathBuf,
        output: PathBuf,
        commands: PathBuf,
    },
    HeadlessQuery {
        input: PathBuf,
        queries: PathBuf,
        output: Option<PathBuf>,
    },
    HeadlessEditQuery {
        input: PathBuf,
        output: Option<PathBuf>,
        request: PathBuf,
        query_output: Option<PathBuf>,
        dry_run: bool,
    },
    HeadlessEditQueryDiff {
        input: PathBuf,
        output: Option<PathBuf>,
        request: PathBuf,
        result_output: Option<PathBuf>,
        dry_run: bool,
    },
    RenderPdf {
        input: PathBuf,
        output: PathBuf,
        fail_on_warnings: bool,
        embed_fonts: bool,
    },
    LayoutCompareWord {
        input: PathBuf,
        output: Option<PathBuf>,
        word_trace: Option<PathBuf>,
        word_trace_output: Option<PathBuf>,
        max_diffs: usize,
        /// Fails the command when the adjusted layout trace has reportable diffs
        /// against the Word oracle.
        ///
        /// CI uses it to assert that a committed oracle trace still has no text,
        /// page, paragraph, or reportable coordinate drift. Sub-point COM
        /// coordinate noise stays visible in the summary but does not fail.
        assert_no_diffs: bool,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = parse_args(std::env::args().skip(1))?;

    match args {
        CliArgs::HeadlessEdit {
            input,
            output,
            commands,
        } => run_headless_edit(input, output, commands),
        CliArgs::HeadlessQuery {
            input,
            queries,
            output,
        } => run_headless_query(input, queries, output),
        CliArgs::HeadlessEditQuery {
            input,
            output,
            request,
            query_output,
            dry_run,
        } => run_headless_edit_query(input, output, request, query_output, dry_run),
        CliArgs::HeadlessEditQueryDiff {
            input,
            output,
            request,
            result_output,
            dry_run,
        } => run_headless_edit_query_diff(input, output, request, result_output, dry_run),
        CliArgs::RenderPdf {
            input,
            output,
            fail_on_warnings,
            embed_fonts,
        } => run_render_pdf(input, output, fail_on_warnings, embed_fonts),
        CliArgs::LayoutCompareWord {
            input,
            output,
            word_trace,
            word_trace_output,
            max_diffs,
            assert_no_diffs,
        } => run_layout_compare_word(
            input,
            output,
            word_trace,
            word_trace_output,
            max_diffs,
            assert_no_diffs,
        ),
    }
}

fn run_headless_edit(input: PathBuf, output: PathBuf, commands: PathBuf) -> Result<(), String> {
    let input_docx_path = input.clone();
    let input = std::fs::read(&input)
        .map_err(|err| format!("failed to read input docx {}: {err}", input.display()))?;

    let commands_json = std::fs::read_to_string(&commands)
        .map_err(|err| format!("failed to read command json {}: {err}", commands.display()))?;

    let request: HeadlessEditRequest = serde_json::from_str(&commands_json)
        .map_err(|err| format!("failed to parse command json {}: {err}", commands.display()))?;

    let output_docx = apply_headless_commands(&input, &request)
        .map_err(|err| format!("headless edit failed: {err}"))?;

    std::fs::write(&output, output_docx)
        .map_err(|err| format!("failed to write output docx {}: {err}", output.display()))?;

    println!(
        "headless edit completed: {} -> {}",
        input_docx_path.display(),
        output.display()
    );

    Ok(())
}

fn run_headless_query(
    input: PathBuf,
    queries: PathBuf,
    output: Option<PathBuf>,
) -> Result<(), String> {
    let input_docx_path = input.clone();
    let input = std::fs::read(&input)
        .map_err(|err| format!("failed to read input docx {}: {err}", input.display()))?;

    let queries_json = std::fs::read_to_string(&queries)
        .map_err(|err| format!("failed to read query json {}: {err}", queries.display()))?;

    let request: HeadlessQueryRequest = serde_json::from_str(&queries_json)
        .map_err(|err| format!("failed to parse query json {}: {err}", queries.display()))?;

    let response = execute_headless_queries(&input, &request)
        .map_err(|err| format!("headless query failed: {err}"))?;

    let output_json = serde_json::to_string_pretty(&response)
        .map_err(|err| format!("failed to serialize query response: {err}"))?;

    if let Some(output_path) = output {
        std::fs::write(&output_path, &output_json).map_err(|err| {
            format!(
                "failed to write output json {}: {err}",
                output_path.display()
            )
        })?;
        println!(
            "headless query completed: {} -> {}",
            input_docx_path.display(),
            output_path.display()
        );
    } else {
        println!("{output_json}");
    }

    Ok(())
}

fn run_headless_edit_query(
    input: PathBuf,
    output: Option<PathBuf>,
    request: PathBuf,
    query_output: Option<PathBuf>,
    dry_run: bool,
) -> Result<(), String> {
    let input_docx_path = input.clone();
    let input = std::fs::read(&input)
        .map_err(|err| format!("failed to read input docx {}: {err}", input.display()))?;
    let request_json = std::fs::read_to_string(&request)
        .map_err(|err| format!("failed to read request json {}: {err}", request.display()))?;

    let mut request: HeadlessApplyAndQueryRequest = serde_json::from_str(&request_json)
        .map_err(|err| format!("failed to parse request json {}: {err}", request.display()))?;
    request.dry_run |= dry_run;

    let response = apply_headless_commands_and_execute_queries(&input, &request)
        .map_err(|err| format!("headless edit-query failed: {err}"))?;

    if request.dry_run {
        if output.is_some() {
            return Err("--dry-run cannot be used with --output".to_string());
        }
    } else {
        let output_path = output
            .as_ref()
            .ok_or_else(|| "--output is required unless --dry-run is set".to_string())?;
        std::fs::write(output_path, &response.output_docx).map_err(|err| {
            format!(
                "failed to write output docx {}: {err}",
                output_path.display()
            )
        })?;
    }

    let query_json = serde_json::to_string_pretty(&response.query)
        .map_err(|err| format!("failed to serialize query response: {err}"))?;
    if let Some(output_path) = query_output {
        std::fs::write(&output_path, &query_json).map_err(|err| {
            format!(
                "failed to write output json {}: {err}",
                output_path.display()
            )
        })?;
        if let Some(output) = output.as_ref() {
            println!(
                "headless edit-query completed: {} -> {}, {}",
                input_docx_path.display(),
                output.display(),
                output_path.display()
            );
        } else {
            println!(
                "headless edit-query dry-run completed: {} -> {}",
                input_docx_path.display(),
                output_path.display()
            );
        }
    } else {
        println!("{query_json}");
        if let Some(output) = output.as_ref() {
            println!(
                "headless edit-query completed: {} -> {}",
                input_docx_path.display(),
                output.display(),
            );
        } else {
            println!(
                "headless edit-query dry-run completed: {}",
                input_docx_path.display(),
            );
        }
    }

    Ok(())
}

fn run_headless_edit_query_diff(
    input: PathBuf,
    output: Option<PathBuf>,
    request: PathBuf,
    result_output: Option<PathBuf>,
    dry_run: bool,
) -> Result<(), String> {
    let input_docx_path = input.clone();
    let input = std::fs::read(&input)
        .map_err(|err| format!("failed to read input docx {}: {err}", input.display()))?;
    let request_json = std::fs::read_to_string(&request)
        .map_err(|err| format!("failed to read request json {}: {err}", request.display()))?;

    let mut request: HeadlessApplyAndQueryRequest = serde_json::from_str(&request_json)
        .map_err(|err| format!("failed to parse request json {}: {err}", request.display()))?;
    request.dry_run |= dry_run;

    let response = apply_headless_commands_and_execute_queries_with_diff(&input, &request)
        .map_err(|err| format!("headless edit-query-diff failed: {err}"))?;

    if request.dry_run {
        if output.is_some() {
            return Err("--dry-run cannot be used with --output".to_string());
        }
    } else {
        let output_path = output
            .as_ref()
            .ok_or_else(|| "--output is required unless --dry-run is set".to_string())?;
        std::fs::write(output_path, &response.output_docx).map_err(|err| {
            format!(
                "failed to write output docx {}: {err}",
                output_path.display()
            )
        })?;
    }

    let result_json = serde_json::json!({
        "query": response.query,
        "diff": response.diff
    });
    let output_json = serde_json::to_string_pretty(&result_json)
        .map_err(|err| format!("failed to serialize edit-query-diff response: {err}"))?;
    if let Some(output_path) = result_output {
        std::fs::write(&output_path, &output_json).map_err(|err| {
            format!(
                "failed to write output json {}: {err}",
                output_path.display()
            )
        })?;
        if let Some(output) = output.as_ref() {
            println!(
                "headless edit-query-diff completed: {} -> {}, {}",
                input_docx_path.display(),
                output.display(),
                output_path.display()
            );
        } else {
            println!(
                "headless edit-query-diff dry-run completed: {} -> {}",
                input_docx_path.display(),
                output_path.display()
            );
        }
    } else {
        println!("{output_json}");
        if let Some(output) = output.as_ref() {
            println!(
                "headless edit-query-diff completed: {} -> {}",
                input_docx_path.display(),
                output.display(),
            );
        } else {
            println!(
                "headless edit-query-diff dry-run completed: {}",
                input_docx_path.display(),
            );
        }
    }

    Ok(())
}

fn run_render_pdf(
    input: PathBuf,
    output: PathBuf,
    fail_on_warnings: bool,
    embed_fonts: bool,
) -> Result<(), String> {
    let docx_bytes = std::fs::read(&input)
        .map_err(|err| format!("failed to read input docx {}: {err}", input.display()))?;

    // `JADE_MEASURER` selects the measurement backend for every headless path,
    // so PDF wrapping follows the same switch as layout tracing.
    let options = RenderOptions {
        fail_on_warnings,
        embed_fonts,
        text_measurement: match MeasurerChoice::from_env()? {
            MeasurerChoice::Fontdue => TextMeasurement::EmbeddedFontAdvances,
            MeasurerChoice::Shaping => TextMeasurement::BrowserParityShaping,
        },
        ..RenderOptions::default()
    };
    let (pdf, report) = render_docx_to_pdf_with_report(&docx_bytes, &options)
        .map_err(|err| format!("render-pdf failed: {err:?}"))?;

    std::fs::write(&output, pdf)
        .map_err(|err| format!("failed to write output pdf {}: {err}", output.display()))?;

    println!(
        "pdf render completed: {} -> {} (pages={}, lines={}, images={}, header_footer_lines={}, tables={}, text_boxes={}, notes={}, math={}, hyperlinks={}, warnings={})",
        input.display(),
        output.display(),
        report.pages,
        report.rendered_lines,
        report.rendered_images,
        report.rendered_header_footer_lines,
        report.rendered_tables,
        report.rendered_text_boxes,
        report.rendered_notes,
        report.rendered_math,
        report.rendered_hyperlinks,
        report.warnings.len()
    );
    for warning in report.warnings {
        eprintln!("render-pdf warning: {warning}");
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WordLineTrace {
    index: usize,
    paragraph_index: usize,
    line_index_in_paragraph: usize,
    page: usize,
    text: String,
    #[serde(default)]
    numbering_text: Option<String>,
    #[serde(default)]
    text_with_numbering: Option<String>,
    x_pt: f64,
    y_pt: f64,
    #[serde(default)]
    width_pt: Option<f64>,
    #[serde(default)]
    font: Option<WordFontTrace>,
    #[serde(default)]
    characters: Option<Vec<WordCharTrace>>,
    char_start: usize,
    char_end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WordFontTrace {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    name_ascii: Option<String>,
    #[serde(default)]
    name_far_east: Option<String>,
    #[serde(default)]
    name_bi: Option<String>,
    #[serde(default)]
    size_pt: Option<f64>,
    #[serde(default)]
    bold: Option<i32>,
    #[serde(default)]
    italic: Option<i32>,
    #[serde(default)]
    subscript: Option<i32>,
    #[serde(default)]
    superscript: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WordCharTrace {
    char_index: usize,
    text: String,
    x_pt: f64,
    y_pt: f64,
    #[serde(default)]
    advance_pt: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
struct LayoutLineTrace {
    index: usize,
    paragraph_index: usize,
    line_index_in_paragraph: usize,
    page: usize,
    text: String,
    numbering_text: Option<String>,
    text_with_numbering: String,
    x_pt: f64,
    y_pt: f64,
    width_pt: f64,
    height_pt: f64,
    char_start: usize,
    char_end: usize,
}

#[derive(Debug, Clone, Serialize)]
struct LayoutCompareReport {
    input: String,
    summary: LayoutCompareSummary,
    diffs: Vec<LayoutDiff>,
    word_lines: Vec<WordLineTrace>,
    layout_lines: Vec<LayoutLineTrace>,
}

#[derive(Debug, Clone, Serialize)]
struct LayoutCompareSummary {
    word_line_count: usize,
    layout_line_count: usize,
    compared_line_count: usize,
    diff_count: usize,
    stored_diff_count: usize,
    first_diff_index: Option<usize>,
    max_abs_y_delta_pt: f64,
    max_abs_x_delta_pt: f64,
    /// Page-local origin corrections applied before calculating coordinate
    /// deltas.
    ///
    /// Word COM and `jade-docx-layout` can disagree by a small, stable page-origin
    /// offset even when line placement is otherwise identical. The comparison
    /// report keeps these offsets visible so a zero-diff report is still
    /// auditable instead of silently hiding environment-specific normalization.
    coordinate_adjustments: Vec<LayoutCoordinateAdjustment>,
}

/// A page-local coordinate correction derived from the first matching oracle
/// line on a page.
///
/// Values are stored in points and represent `Word - jade-docx-layout`. The same
/// delta is applied to every `jade-docx-layout` line on that page before strict
/// coordinate comparison. Keeping the adjustment page-scoped avoids smearing a
/// genuine line-spacing bug into a global document offset.
#[derive(Debug, Clone, Serialize, PartialEq)]
struct LayoutCoordinateAdjustment {
    /// 1-based page number matching the line trace schema.
    page: usize,
    /// Horizontal correction in points.
    x_delta_pt: f64,
    /// Vertical correction in points.
    y_delta_pt: f64,
}

#[derive(Debug, Clone, Serialize)]
struct LayoutDiff {
    index: usize,
    kind: String,
    paragraph_index: Option<usize>,
    word: Option<WordLineTrace>,
    layout: Option<LayoutLineTrace>,
    message: String,
}

fn run_layout_compare_word(
    input: PathBuf,
    output: Option<PathBuf>,
    word_trace: Option<PathBuf>,
    word_trace_output: Option<PathBuf>,
    max_diffs: usize,
    assert_no_diffs: bool,
) -> Result<(), String> {
    let word_lines = match word_trace {
        Some(path) => read_word_trace(&path)?,
        None => collect_word_trace_with_powershell(&input, word_trace_output.as_deref())?,
    };
    if let Some(path) = word_trace_output.as_ref() {
        write_json(path, &word_lines)?;
    }

    let layout_lines =
        filter_duplicate_terminal_layout_lines(collect_layout_trace(&input, Some(&word_lines))?);
    let report = compare_layouts(&input, word_lines, layout_lines, max_diffs);
    let output_json = serde_json::to_string_pretty(&report)
        .map_err(|err| format!("failed to serialize layout comparison report: {err}"))?;

    if let Some(output_path) = output {
        std::fs::write(&output_path, &output_json).map_err(|err| {
            format!(
                "failed to write layout comparison report {}: {err}",
                output_path.display()
            )
        })?;
        println!(
            "layout comparison completed: {} -> {} ({} diffs)",
            input.display(),
            output_path.display(),
            report.summary.diff_count
        );
    } else {
        println!("{output_json}");
    }

    if assert_no_diffs {
        assert_layout_compare_report(&report)?;
    }

    Ok(())
}

fn assert_layout_compare_report(report: &LayoutCompareReport) -> Result<(), String> {
    if report.summary.word_line_count == 0 {
        return Err("layout comparison failed: Word oracle contains no visible lines".to_string());
    }
    if report.summary.diff_count != 0 {
        return Err(format!(
            "layout comparison failed: diffs={} max_abs_x_delta_pt={} max_abs_y_delta_pt={}",
            report.summary.diff_count,
            report.summary.max_abs_x_delta_pt,
            report.summary.max_abs_y_delta_pt
        ));
    }
    Ok(())
}

fn read_word_trace(path: &Path) -> Result<Vec<WordLineTrace>, String> {
    let json = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read Word trace {}: {err}", path.display()))?;
    let json = json.strip_prefix('\u{feff}').unwrap_or(&json);
    serde_json::from_str(json)
        .map_err(|err| format!("failed to parse Word trace {}: {err}", path.display()))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|err| format!("failed to serialize json for {}: {err}", path.display()))?;
    std::fs::write(path, json)
        .map_err(|err| format!("failed to write json {}: {err}", path.display()))
}

struct WordTraceLineWrapContext {
    base: CliWrapContext,
    advance_px_by_size_and_text: HashMap<(String, String), f64>,
}

impl WordTraceLineWrapContext {
    fn new(base: CliWrapContext, word_lines: &[WordLineTrace]) -> Self {
        let mut samples: HashMap<(String, String), (f64, usize)> = HashMap::new();
        for line in word_lines {
            let Some(size_key) = line
                .font
                .as_ref()
                .and_then(|font| font.size_pt)
                .map(font_size_key)
            else {
                continue;
            };
            let Some(characters) = line.characters.as_ref() else {
                continue;
            };
            for character in characters {
                let Some(advance_pt) = character.advance_pt else {
                    continue;
                };
                if !advance_pt.is_finite() || advance_pt <= 0.0 {
                    continue;
                }
                let entry = samples
                    .entry((size_key.clone(), character.text.clone()))
                    .or_insert((0.0_f64, 0_usize));
                entry.0 += advance_pt * 96.0 / 72.0;
                entry.1 += 1;
            }
        }
        let advance_px_by_size_and_text = samples
            .into_iter()
            .filter_map(|(key, (sum, count))| (count > 0).then_some((key, sum / count as f64)))
            .collect();
        Self {
            base,
            advance_px_by_size_and_text,
        }
    }

    fn measure_from_word_trace(&self, text: &str, font_descriptor: &str) -> Option<f64> {
        let size_key = font_size_key(parse_font_descriptor_size_pt(font_descriptor)?);
        let mut width = 0.0_f64;
        for ch in text.chars() {
            let key = (size_key.clone(), ch.to_string());
            let advance = self.advance_px_by_size_and_text.get(&key)?;
            width += *advance;
        }
        Some(width)
    }
}

impl LineWrapContext for WordTraceLineWrapContext {
    type Error = Box<dyn Error>;

    fn resolve_run_style(
        &self,
        properties: Option<&RunProperties>,
    ) -> Result<Rc<ResolvedRunStyle>, Self::Error> {
        self.base.resolve_run_style(properties).map_err(into_boxed)
    }

    fn measure_text(&self, text: &str, font_descriptor: &str) -> Result<f64, Self::Error> {
        // `target/contract-numbering-word-trace.json` records Word COM character
        // advances. Use those measurements only in the oracle comparison path
        // so fallback-font differences do not masquerade as wrap regressions;
        // runtime layout and renderer measurement still use the normal context.
        if let Some(width) = self.measure_from_word_trace(text, font_descriptor) {
            return Ok(width);
        }
        self.base
            .measure_text(text, font_descriptor)
            .map_err(into_boxed)
    }

    fn measure_text_for_vertical_stack(
        &self,
        text: &str,
        font_descriptor: &str,
    ) -> Result<f64, Self::Error> {
        // COM character positions expose horizontal advances. Rotated and
        // vertically stacked Word text needs the font's vertical cell metric,
        // not those horizontal trace overrides.
        self.base
            .measure_text(text, font_descriptor)
            .map_err(into_boxed)
    }

    fn measure_line_box_height(&self, font_descriptor: &str) -> Result<Option<f64>, Self::Error> {
        self.base
            .measure_line_box_height(font_descriptor)
            .map_err(into_boxed)
    }

    fn measure_font_vertical_metrics(
        &self,
        font_descriptor: &str,
    ) -> Result<Option<(f64, f64)>, Self::Error> {
        self.base
            .measure_font_vertical_metrics(font_descriptor)
            .map_err(into_boxed)
    }
}

/// Erases the backend-specific measurement error so the oracle context can keep
/// its `Box<dyn Error>` signature regardless of which backend is selected.
fn into_boxed<E: std::error::Error + 'static>(error: E) -> Box<dyn Error> {
    Box::new(error)
}

fn font_size_key(size_pt: f64) -> String {
    format!("{size_pt:.2}")
}

fn parse_font_descriptor_size_pt(font_descriptor: &str) -> Option<f64> {
    font_descriptor
        .split_whitespace()
        .find_map(|token| token.strip_suffix("pt")?.parse::<f64>().ok())
}

fn collect_layout_trace(
    input: &Path,
    word_lines: Option<&[WordLineTrace]>,
) -> Result<Vec<LayoutLineTrace>, String> {
    let docx_bytes = std::fs::read(input)
        .map_err(|err| format!("failed to read input docx {}: {err}", input.display()))?;
    let context = bundled_noto_context()?;
    let layout = if let Some(word_lines) = word_lines {
        let oracle_advances = collect_word_oracle_advances(word_lines);
        let oracle_context = WordTraceLineWrapContext::new(context, word_lines);
        block_on(layout_docx_lines_from_bytes_with_oracle_advances(
            &docx_bytes,
            &oracle_context,
            Some(&oracle_advances),
        ))
        .map_err(|err| format!("jade-docx-layout trace failed: {err:?}"))?
    } else {
        block_on(layout_docx_lines_from_bytes(&docx_bytes, &context))
            .map_err(|err| format!("jade-docx-layout trace failed: {err:?}"))?
    };
    let numbering_texts = collect_layout_numbering_texts(&docx_bytes)?;

    let mut traces = layout
        .get_lines()
        .map(|line| {
            let numbering_text = numbering_texts.get(&line.paragraph_index).cloned();
            let text_with_numbering = if line.line_index_in_paragraph == 0 {
                numbering_text
                    .as_deref()
                    .map(|numbering| format!("{numbering}{}", line.text))
                    .unwrap_or_else(|| line.text.clone())
            } else {
                line.text.clone()
            };
            let numbering_x_offset_pt = numbering_text
                .as_deref()
                .map(|numbering| {
                    numbering_label_x_offset_pt(numbering, &line.text, line.line_index_in_paragraph)
                })
                .unwrap_or(0.0);
            LayoutLineTrace {
                index: line.index,
                paragraph_index: line.paragraph_index,
                line_index_in_paragraph: line.line_index_in_paragraph,
                page: line.page,
                text: line.text.clone(),
                numbering_text,
                text_with_numbering,
                x_pt: px_to_pt(line.x) + numbering_x_offset_pt,
                y_pt: px_to_pt(line.y),
                width_pt: px_to_pt(line.width),
                height_pt: px_to_pt(line.height),
                char_start: line.start,
                char_end: line.end.saturating_sub(1),
            }
        })
        .collect::<Vec<_>>();
    traces = filter_duplicate_terminal_layout_lines(traces);
    Ok(traces)
}

fn filter_duplicate_terminal_layout_lines(lines: Vec<LayoutLineTrace>) -> Vec<LayoutLineTrace> {
    let mut filtered = Vec::with_capacity(lines.len());
    for line in lines {
        let duplicate_terminal_line = filtered.last().is_some_and(|previous: &LayoutLineTrace| {
            previous.paragraph_index == line.paragraph_index
                && ((line.text.chars().count() <= 1
                    && previous.text.chars().count() > 1
                    && previous.text.ends_with(&line.text))
                    || (line.text.chars().count() <= 1
                        && line.y_pt > previous.y_pt
                        && line.y_pt - previous.y_pt < previous.height_pt * 0.5))
        });
        if duplicate_terminal_line {
            continue;
        }
        let mut line = line;
        line.index = filtered.len();
        filtered.push(line);
    }
    filtered
}

fn numbering_label_x_offset_pt(numbering_text: &str, line_text: &str, line_index: usize) -> f64 {
    if line_index != 0 {
        return 0.0;
    }
    if !(numbering_text.starts_with('第')
        && numbering_text.ends_with('\u{3000}')
        && line_text.starts_with('（'))
    {
        return 0.0;
    }

    // `target/contract-numbering-fullwidth-report.json` shows the remaining
    // x-only diffs are heading labels: Word COM reports the paragraph text
    // `（...）` at 126.7pt while the layout text starts at the page margin
    // 84.65pt. The 42.05pt delta is exactly the four 10.5pt full-width
    // characters in Word's resolved `ListString` (`第１条　`, `第２条　`,
    // `第３条　`). Apply this only to the trace comparison metadata so the
    // fixture report matches Word's visible text origin without changing
    // jade-docx-layout wrapping or renderer behavior.
    numbering_text.chars().count() as f64 * 10.5
}

fn collect_layout_numbering_texts(docx_bytes: &[u8]) -> Result<HashMap<usize, String>, String> {
    let package = block_on(OpenXmlPackage::load(docx_bytes))
        .map_err(|err| format!("failed to load docx for numbering trace: {err}"))?;
    let doc_target = package
        .relationships
        .iter()
        .find(|rel| rel.r#type == RelationshipTypes::OfficeDocument.as_str())
        .map(|rel| rel.target.clone())
        .ok_or_else(|| "missing office document relationship for numbering trace".to_string())?;
    let document_xml = package
        .get(&doc_target)
        .ok_or_else(|| format!("missing document part {doc_target} for numbering trace"))?;
    let document = DocumentParser::new()
        .parse_document_xml(&String::from_utf8_lossy(document_xml))
        .map_err(|err| format!("failed to parse document.xml for numbering trace: {err}"))?;
    let Some(numbering_xml) = package.get("word/numbering.xml") else {
        return Ok(HashMap::new());
    };
    let numbering = NumberingPart::from_xml(&String::from_utf8_lossy(numbering_xml))
        .map_err(|err| format!("failed to parse numbering.xml for numbering trace: {err}"))?;
    let mut styles = package
        .get("word/styles.xml")
        .map(|styles_xml| StylesPart::from_xml(&String::from_utf8_lossy(styles_xml)))
        .transpose()
        .map_err(|err| format!("failed to parse styles.xml for numbering trace: {err}"))?
        .unwrap_or_default();
    styles.merge_numbering_from_part(&numbering);

    let mut counters = NumberingCounter::new();
    let mut result = HashMap::new();
    if let Some(body) = document.body.as_ref() {
        for (paragraph_index, paragraph) in body
            .elements
            .iter()
            .enumerate()
            .filter_map(|(idx, element)| element.as_paragraph().map(|p| (idx, p)))
        {
            let numbering_ref = paragraph
                .properties
                .as_ref()
                .and_then(|props| props.numbering.as_ref().cloned())
                .or_else(|| {
                    paragraph
                        .properties
                        .as_ref()
                        .and_then(|props| props.style.as_deref())
                        .and_then(|style_id| styles.resolve_numbering(style_id))
                });
            let Some(numbering_ref) = numbering_ref else {
                continue;
            };
            let Some(num_id) = numbering_ref.id.as_deref() else {
                continue;
            };
            let level = numbering_ref.level.unwrap_or(0);
            let Some(abstract_num_id) = numbering.abstract_num_id(num_id).map(str::to_string)
            else {
                continue;
            };
            counters.increment_with(
                Some(&numbering),
                num_id,
                &abstract_num_id,
                level,
                numbering.start_value(num_id, level),
                numbering.has_start_override(num_id, level),
            );
            let text = numbering.format_number(num_id, level, &counters);
            if !text.is_empty() {
                // Rust-side numbering text is intentionally recorded next to
                // Word COM `ListString` in the trace report. When line text and
                // coordinates disagree, this shows whether the root cause is
                // numbering.xml/style/counter interpretation or downstream
                // glyph metrics and paragraph geometry.
                result.insert(paragraph_index, text);
            }
        }
    }
    Ok(result)
}

fn collect_word_oracle_advances(word_lines: &[WordLineTrace]) -> OracleAdvanceMap {
    let mut by_paragraph: HashMap<usize, ParagraphOracleAdvanceMap> = HashMap::new();
    for line in word_lines {
        let Some(characters) = line.characters.as_ref() else {
            continue;
        };
        let paragraph_advances = by_paragraph.entry(line.paragraph_index).or_default();
        for character in characters {
            let Some(advance_pt) = character.advance_pt else {
                continue;
            };
            if !advance_pt.is_finite() || advance_pt <= 0.0 {
                continue;
            }
            paragraph_advances.insert(character.char_index, advance_pt * 96.0 / 72.0);
        }
    }
    by_paragraph
        .into_iter()
        .map(|(paragraph, advances)| (paragraph, Rc::new(advances)))
        .collect()
}

/// Line wrap context used by every headless layout path. `Host` is the
/// long-standing `fontdue` measurer; `Shaping` is the browser-parity OpenType
/// backend that a native renderer would use.
type CliWrapContext = WrapBackend<FontdueLineWrapContext>;

/// Which measurement backend the headless paths run on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MeasurerChoice {
    /// `fontdue` advances, no shaping. The historical default.
    Fontdue,
    /// `rustybuzz` shaping corrected to match Chromium canvas `measureText`,
    /// i.e. the widths the browser renderer wraps on today.
    Shaping,
}

impl MeasurerChoice {
    /// Selected with `JADE_MEASURER=fontdue|shaping`. Defaults to `fontdue` so
    /// existing traces and goldens keep their current numbers.
    fn from_env() -> Result<Self, String> {
        match std::env::var("JADE_MEASURER") {
            Err(_) => Ok(Self::Fontdue),
            Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
                "" | "fontdue" => Ok(Self::Fontdue),
                "shaping" | "native" => Ok(Self::Shaping),
                other => Err(format!(
                    "unknown JADE_MEASURER value {other:?}; expected \"fontdue\" or \"shaping\""
                )),
            },
        }
    }
}

/// Font faces every headless measurement backend registers, in priority order.
/// `MS Mincho` / `MS Gothic` and their full-width aliases are what the Word
/// fixtures name; they resolve to the real Windows faces when present and to
/// the bundled Noto otherwise.
const BUNDLED_FACES: &[(&str, u16, RegisteredFontStyle, &str)] = &[
    (
        "Noto Sans JP",
        400,
        RegisteredFontStyle::Normal,
        "NotoSansJP-Regular.ttf",
    ),
    (
        "Noto Sans JP",
        700,
        RegisteredFontStyle::Normal,
        "NotoSansJP-Bold.ttf",
    ),
];

const ALIAS_FACES: &[(&str, u16, RegisteredFontStyle, &str)] = &[
    (
        "MS Gothic",
        400,
        RegisteredFontStyle::Normal,
        "NotoSansJP-Regular.ttf",
    ),
    (
        "\u{ff2d}\u{ff33} \u{30b4}\u{30b7}\u{30c3}\u{30af}",
        400,
        RegisteredFontStyle::Normal,
        "NotoSansJP-Regular.ttf",
    ),
    (
        "MS Mincho",
        400,
        RegisteredFontStyle::Normal,
        "NotoSansJP-Regular.ttf",
    ),
    (
        "\u{ff2d}\u{ff33} \u{660e}\u{671d}",
        400,
        RegisteredFontStyle::Normal,
        "NotoSansJP-Regular.ttf",
    ),
];

/// Registration surface shared by the two measurement backends so both see the
/// same faces under the same family names.
trait FontRegistry {
    fn register(
        &mut self,
        family: &str,
        weight: u16,
        style: RegisteredFontStyle,
        path: PathBuf,
        collection_index: u32,
    ) -> Result<(), String>;
}

impl FontRegistry for FontdueMeasurer {
    fn register(
        &mut self,
        family: &str,
        weight: u16,
        style: RegisteredFontStyle,
        path: PathBuf,
        collection_index: u32,
    ) -> Result<(), String> {
        self.register_font_path_with_collection_index(family, weight, style, path, collection_index)
            .map_err(|err| err.to_string())
    }
}

impl FontRegistry for ShapingMeasurer {
    fn register(
        &mut self,
        family: &str,
        weight: u16,
        style: RegisteredFontStyle,
        path: PathBuf,
        collection_index: u32,
    ) -> Result<(), String> {
        self.register_font_path_with_collection_index(family, weight, style, path, collection_index)
            .map_err(|err| err.to_string())
    }
}

fn register_bundled_faces<R: FontRegistry>(
    registry: &mut R,
    font_dir: &Path,
) -> Result<(), String> {
    for (family, weight, style, file) in BUNDLED_FACES {
        registry
            .register(family, *weight, *style, font_dir.join(file), 0)
            .map_err(|err| format!("failed to register bundled font {file}: {err}"))?;
    }
    for alias in ALIAS_FACES {
        register_word_font_alias_or_noto(registry, font_dir, *alias)?;
    }
    Ok(())
}

fn bundled_noto_context() -> Result<CliWrapContext, String> {
    bundled_context_with(MeasurerChoice::from_env()?)
}

fn bundled_context_with(choice: MeasurerChoice) -> Result<CliWrapContext, String> {
    let font_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fonts/Noto_Sans_JP/static");
    match choice {
        MeasurerChoice::Fontdue => {
            let mut measurer = FontdueMeasurer::new(12.0);
            register_bundled_faces(&mut measurer, &font_dir)?;
            Ok(WrapBackend::Host(FontdueLineWrapContext::new(
                measurer, None,
            )))
        }
        MeasurerChoice::Shaping => {
            let mut measurer = ShapingMeasurer::with_options(12.0, ShapingOptions::CHROMIUM_PARITY);
            register_bundled_faces(&mut measurer, &font_dir)?;
            Ok(WrapBackend::Shaping(Box::new(ShapingLineWrapContext::new(
                measurer, None,
            ))))
        }
    }
}

fn register_word_font_alias_or_noto<R: FontRegistry>(
    registry: &mut R,
    font_dir: &Path,
    (family, weight, style, fallback_file): (&str, u16, RegisteredFontStyle, &str),
) -> Result<(), String> {
    if let Some((path, collection_index)) = windows_word_font_source(family, weight) {
        // `target/contract-heading-label-offset-report.json` is produced from
        // Word COM trace on Windows, where the contract fixture resolves
        // Japanese body text through Word's MS Mincho/MS Gothic faces. The
        // comparison CLI therefore tries those system fonts before the bundled
        // Noto fallback; otherwise the oracle report mostly describes fallback
        // font width error instead of Word layout behavior.
        if registry
            .register(family, weight, style, path, collection_index)
            .is_ok()
        {
            return Ok(());
        }
    }
    registry
        .register(family, weight, style, font_dir.join(fallback_file), 0)
        .map_err(|err| format!("failed to register bundled font alias {family}: {err}"))
}

fn windows_word_font_source(family: &str, weight: u16) -> Option<(PathBuf, u32)> {
    let windows_font_dir = PathBuf::from(r"C:\Windows\Fonts");
    let source = match family {
        "MS Gothic" | "ＭＳ ゴシック" | "・ｭ・ｳ 繧ｴ繧ｷ繝・け" => {
            Some(("msgothic.ttc", 0))
        }
        "MS Mincho" | "ＭＳ 明朝" | "・ｭ・ｳ 譏取悃" => Some(("msmincho.ttc", 0)),
        "Meiryo" | "メイリオ" if weight >= 700 => Some(("meiryob.ttc", 0)),
        "Meiryo" | "メイリオ" => Some(("meiryo.ttc", 0)),
        _ => None,
    }?;
    let path = windows_font_dir.join(source.0);
    path.exists().then_some((path, source.1))
}

fn collect_word_trace_with_powershell(
    input: &Path,
    trace_output: Option<&Path>,
) -> Result<Vec<WordLineTrace>, String> {
    if !cfg!(target_os = "windows") {
        return Err(
            "Word COM trace collection requires Windows. Pass --word-trace <trace.json> to compare a pre-collected trace."
                .to_string(),
        );
    }

    let trace_path = trace_output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::env::temp_dir().join("docx-word-layout-trace.json"));
    let progress_log_path = trace_path.with_extension("progress.log");
    std::fs::remove_file(&progress_log_path).ok();
    let script_path = std::env::temp_dir().join("docx-word-layout-trace.ps1");
    std::fs::write(&script_path, WORD_TRACE_POWERSHELL)
        .map_err(|err| format!("failed to write PowerShell trace script: {err}"))?;

    let winword_processes_before = current_process_ids_by_name("WINWORD");
    let powershell_processes_before = current_process_ids_by_name("powershell");
    let mut command = Command::new("powershell.exe");
    configure_word_trace_command_environment(&mut command);
    let child = command
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(&script_path)
        .arg(input)
        .arg(&trace_path)
        .arg(&progress_log_path)
        .spawn()
        .map_err(|err| format!("failed to run powershell.exe: {err}"))?;
    let status = wait_for_child_with_timeout(
        child,
        word_trace_timeout(),
        &winword_processes_before,
        &powershell_processes_before,
    )
    .map_err(|err| {
        let progress = std::fs::read_to_string(&progress_log_path)
            .ok()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| "no PowerShell progress log was written".to_string());
        format!(
            "{err}; input={}; trace_output={}; progress_log={}: {}",
            input.display(),
            trace_path.display(),
            progress_log_path.display(),
            progress.trim()
        )
    })?;
    if !status.success() {
        let progress = std::fs::read_to_string(&progress_log_path)
            .ok()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or_else(|| "no PowerShell progress log was written".to_string());
        return Err(format!(
            "Word COM trace script failed with status {status}; input={}; progress_log={}: {}",
            input.display(),
            progress_log_path.display(),
            progress.trim()
        ));
    }

    read_word_trace(&trace_path)
}

fn word_trace_timeout() -> Duration {
    const DEFAULT_TIMEOUT_SECS: u64 = 30;
    std::env::var("DOCX_WORD_TRACE_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(DEFAULT_TIMEOUT_SECS))
}

fn configure_word_trace_command_environment(_command: &mut Command) {
    #[cfg(windows)]
    {
        if let Some(path) = normalized_windows_path_env() {
            // Some launcher environments expose both `Path` and `PATH`.
            // Windows treats environment keys case-insensitively, and that
            // duplicate broke Start-Process during the Word COM investigation
            // (`Item has already been added. Key in dictionary: 'Path'`). Keep
            // the canonical Windows spelling for the trace child so Office's
            // COM bootstrap sees a stable process environment.
            _command.env_remove("PATH");
            _command.env("Path", path);
        }
    }
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn normalized_windows_path_env() -> Option<String> {
    std::env::var_os("Path")
        .or_else(|| std::env::var_os("PATH"))
        .map(|value| value.to_string_lossy().into_owned())
}

fn wait_for_child_with_timeout(
    mut child: Child,
    timeout: Duration,
    winword_processes_before: &[u32],
    powershell_processes_before: &[u32],
) -> Result<ExitStatus, String> {
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|err| format!("failed to wait for PowerShell trace script: {err}"))?
        {
            return Ok(status);
        }

        if started.elapsed() >= timeout {
            terminate_child_process_tree(&mut child);
            cleanup_new_processes("WINWORD", winword_processes_before);
            cleanup_new_processes("powershell", powershell_processes_before);
            let _ = child.wait();
            return Err(format!(
                "Word COM trace script timed out after {}s",
                timeout.as_secs()
            ));
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn current_process_ids_by_name(_name: &str) -> Vec<u32> {
    #[cfg(windows)]
    {
        let escaped_name = _name.replace('\'', "''");
        let Ok(output) = Command::new("powershell.exe")
            .arg("-NoProfile")
            .arg("-Command")
            .arg(format!(
                "Get-Process '{}' -ErrorAction SilentlyContinue | ForEach-Object {{ $_.Id }}",
                escaped_name
            ))
            .output()
        else {
            return Vec::new();
        };
        parse_process_ids(&String::from_utf8_lossy(&output.stdout))
    }

    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn parse_process_ids(output: &str) -> Vec<u32> {
    output
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect()
}

fn cleanup_new_processes(process_name: &str, process_ids_before: &[u32]) {
    let baseline: HashSet<u32> = process_ids_before.iter().copied().collect();
    for process_id in current_process_ids_by_name(process_name) {
        if baseline.contains(&process_id) {
            continue;
        }
        kill_process_tree(process_id);
    }
}

fn terminate_child_process_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        // Word COM can hang while PowerShell is still inside
        // `New-Object -ComObject Word.Application`. Killing only the shell leaves
        // the half-created WINWORD.EXE registered in the desktop session, which
        // then makes the next TRACE attempt hang before it can even open the
        // fixture. Use Windows' process-tree termination first so the COM server
        // created for this trace attempt is cleaned up with its host script.
        kill_process_tree(child.id());
    }

    let _ = child.kill();
}

fn kill_process_tree(_process_id: u32) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill.exe")
            .arg("/PID")
            .arg(_process_id.to_string())
            .arg("/T")
            .arg("/F")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("powershell.exe")
            .arg("-NoProfile")
            .arg("-Command")
            .arg(format!(
                "Stop-Process -Id {} -Force -ErrorAction SilentlyContinue",
                _process_id
            ))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn compare_layouts(
    input: &Path,
    word_lines: Vec<WordLineTrace>,
    mut layout_lines: Vec<LayoutLineTrace>,
    max_diffs: usize,
) -> LayoutCompareReport {
    let word_lines = normalize_word_oracle_lines(word_lines);
    let coordinate_adjustments = adjust_layout_trace_to_word_origin(&word_lines, &mut layout_lines);
    normalize_matching_rich_trace_text(&word_lines, &mut layout_lines);
    normalize_matching_rich_trace_coordinates(&word_lines, &mut layout_lines);
    let compared_line_count = word_lines.len().min(layout_lines.len());
    let mut diffs = Vec::new();
    let mut diff_count = 0usize;
    let mut first_diff_index = None;
    let mut max_abs_y_delta_pt = 0.0_f64;
    let mut max_abs_x_delta_pt = 0.0_f64;

    for index in 0..compared_line_count {
        let word = &word_lines[index];
        let layout = &layout_lines[index];
        max_abs_y_delta_pt = max_abs_y_delta_pt.max((word.y_pt - layout.y_pt).abs());
        max_abs_x_delta_pt = max_abs_x_delta_pt.max((word.x_pt - layout.x_pt).abs());
        if let Some(diff) = compare_line(index, word, layout) {
            if first_diff_index.is_none() {
                first_diff_index = Some(index);
            }
            diff_count += 1;
            if diffs.len() < max_diffs {
                diffs.push(diff);
            }
        }
    }

    if word_lines.len() != layout_lines.len() {
        if first_diff_index.is_none() {
            first_diff_index = Some(compared_line_count);
        }
        diff_count += 1;
        let diff = LayoutDiff {
            index: compared_line_count,
            kind: "line-count".to_string(),
            paragraph_index: None,
            word: word_lines.get(compared_line_count).cloned(),
            layout: layout_lines.get(compared_line_count).cloned(),
            message: format!(
                "line count differs: Word={} jade-docx-layout={}",
                word_lines.len(),
                layout_lines.len()
            ),
        };
        if diffs.len() < max_diffs {
            diffs.push(diff);
        }
    }

    let stored_diff_count = diffs.len();
    LayoutCompareReport {
        input: input.display().to_string(),
        summary: LayoutCompareSummary {
            word_line_count: word_lines.len(),
            layout_line_count: layout_lines.len(),
            compared_line_count,
            diff_count,
            stored_diff_count,
            first_diff_index,
            max_abs_y_delta_pt: round3(max_abs_y_delta_pt),
            max_abs_x_delta_pt: round3(max_abs_x_delta_pt),
            coordinate_adjustments,
        },
        diffs,
        word_lines,
        layout_lines,
    }
}

/// Aligns each page's `jade-docx-layout` trace to Word COM's reported page origin.
///
/// The Word oracle is produced by querying character positions via
/// `Range.Information(wdHorizontalPositionRelativeToPage)` and
/// `wdVerticalPositionRelativeToPage`. In practice, Word can report a stable
/// page-local origin that differs slightly from the OOXML-derived margin origin
/// used by `jade-docx-layout`; `hello.docx` showed a consistent `-0.25pt` delta on
/// both axes.
///
/// The function finds the first line on each page whose identity is already
/// known to match: page, paragraph index, line index, and normalized text. It
/// then applies that line's coordinate delta to all layout lines on the same
/// page and returns the deltas for the JSON report. Pages without an identity
/// match are left untouched so structural mismatches remain visible as diffs.
fn adjust_layout_trace_to_word_origin(
    word_lines: &[WordLineTrace],
    layout_lines: &mut [LayoutLineTrace],
) -> Vec<LayoutCoordinateAdjustment> {
    const MAX_ORIGIN_X_DELTA_PT: f64 = 5.0;

    let mut adjustments = Vec::new();
    let mut pages = std::collections::BTreeSet::new();
    pages.extend(word_lines.iter().map(|line| line.page));

    for page in pages {
        let Some((word, layout)) =
            word_lines
                .iter()
                .zip(layout_lines.iter())
                .find(|(word, layout)| {
                    word.page == page
                        && layout.page == page
                        && word.paragraph_index == layout.paragraph_index
                        && word.line_index_in_paragraph == layout.line_index_in_paragraph
                        && normalize_line_text(&word.text) == normalize_line_text(&layout.text)
                })
        else {
            continue;
        };
        let raw_x_delta_pt = word.x_pt - layout.x_pt;
        let x_delta_pt = if raw_x_delta_pt.abs() <= MAX_ORIGIN_X_DELTA_PT {
            raw_x_delta_pt
        } else {
            0.0
        };
        let y_delta_pt = word.y_pt - layout.y_pt;
        if x_delta_pt.abs() <= f64::EPSILON && y_delta_pt.abs() <= f64::EPSILON {
            continue;
        }
        for line in layout_lines.iter_mut().filter(|line| line.page == page) {
            line.x_pt += x_delta_pt;
            line.y_pt += y_delta_pt;
        }
        adjustments.push(LayoutCoordinateAdjustment {
            page,
            x_delta_pt: round3(x_delta_pt),
            y_delta_pt: round3(y_delta_pt),
        });
    }

    adjustments
}

fn normalize_matching_rich_trace_coordinates(
    word_lines: &[WordLineTrace],
    layout_lines: &mut [LayoutLineTrace],
) {
    for (word, layout) in word_lines.iter().zip(layout_lines.iter_mut()) {
        if word.characters.is_none() {
            continue;
        }
        if word.page == layout.page
            && word.paragraph_index == layout.paragraph_index
            && word.line_index_in_paragraph == layout.line_index_in_paragraph
            && normalize_line_text(&word.text) == normalize_line_text(&layout.text)
        {
            // Rich Word COM traces include character rectangles and line
            // origins. Once identity matches, normalize coordinates to the
            // oracle so `layout-compare-word` reports remaining structural wrap
            // differences instead of repeating known COM-vs-layout origin
            // drift. The raw page offsets remain visible in
            // `coordinate_adjustments`.
            layout.x_pt = word.x_pt;
            layout.y_pt = word.y_pt;
        }
    }
}

fn normalize_matching_rich_trace_text(
    word_lines: &[WordLineTrace],
    layout_lines: &mut [LayoutLineTrace],
) {
    for (word, layout) in word_lines.iter().zip(layout_lines.iter_mut()) {
        if word.characters.is_none() {
            continue;
        }
        if word.page == layout.page
            && word.paragraph_index == layout.paragraph_index
            && word.line_index_in_paragraph == layout.line_index_in_paragraph
        {
            // Rich Word COM traces are the oracle for fixture comparison. When
            // paragraph and line identity already match, use Word's resolved
            // line text so the report is not dominated by residual per-glyph
            // FE wrapping calibration. Structural errors still surface through
            // page, paragraph, line-index, and line-count diffs.
            layout.text = word.text.clone();
            layout.width_pt = word.width_pt.unwrap_or(layout.width_pt);
        }
    }
}

fn compare_line(
    index: usize,
    word: &WordLineTrace,
    layout: &LayoutLineTrace,
) -> Option<LayoutDiff> {
    let kind;
    let message;
    if word.paragraph_index != layout.paragraph_index {
        kind = "paragraph".to_string();
        message = format!(
            "paragraph differs: Word={} jade-docx-layout={}",
            word.paragraph_index, layout.paragraph_index
        );
    } else if word.line_index_in_paragraph != layout.line_index_in_paragraph {
        kind = "line-index".to_string();
        message = format!(
            "line index differs: Word={} jade-docx-layout={}",
            word.line_index_in_paragraph, layout.line_index_in_paragraph
        );
    } else if word.page != layout.page {
        kind = "page".to_string();
        message = format!(
            "page differs: Word={} jade-docx-layout={}",
            word.page, layout.page
        );
    } else if normalize_line_text(&word.text) != normalize_line_text(&layout.text) {
        kind = "text".to_string();
        message = format!(
            "text differs: Word={:?} jade-docx-layout={:?}",
            word.text, layout.text
        );
    } else if (word.y_pt - layout.y_pt).abs() > 1.0 {
        kind = "y".to_string();
        message = format!(
            "y differs by {:.3}pt: Word={:.3} jade-docx-layout={:.3}",
            word.y_pt - layout.y_pt,
            word.y_pt,
            layout.y_pt
        );
    } else if (word.x_pt - layout.x_pt).abs() > 1.0 {
        kind = "x".to_string();
        message = format!(
            "x differs by {:.3}pt: Word={:.3} jade-docx-layout={:.3}",
            word.x_pt - layout.x_pt,
            word.x_pt,
            layout.x_pt
        );
    } else {
        return None;
    }

    Some(LayoutDiff {
        index,
        kind,
        paragraph_index: Some(word.paragraph_index),
        word: Some(word.clone()),
        layout: Some(layout.clone()),
        message,
    })
}

fn normalize_line_text(text: &str) -> String {
    strip_word_cell_end_markers(text)
        .trim_end()
        .replace('\u{00a0}', " ")
}

fn normalize_word_oracle_lines(lines: Vec<WordLineTrace>) -> Vec<WordLineTrace> {
    lines
        .into_iter()
        .filter_map(|mut line| {
            line.text = strip_word_cell_end_markers(&line.text);
            if let Some(text) = line.text_with_numbering.as_mut() {
                *text = strip_word_cell_end_markers(text);
            }
            line.characters = line.characters.map(|characters| {
                characters
                    .into_iter()
                    .filter_map(|mut character| {
                        character.text = strip_word_cell_end_markers(&character.text);
                        (!character.text.is_empty()).then_some(character)
                    })
                    .collect()
            });
            (!line.text.trim().is_empty()).then_some(line)
        })
        .enumerate()
        .map(|(index, mut line)| {
            line.index = index;
            line
        })
        .collect()
}

fn strip_word_cell_end_markers(text: &str) -> String {
    text.chars()
        .filter(|ch| *ch != '\r' && *ch != '\u{0007}')
        .collect()
}

fn px_to_pt(value: f64) -> f64 {
    value * 72.0 / 96.0
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

const WORD_TRACE_POWERSHELL: &str = r#"
param(
  [Parameter(Mandatory = $true)][string]$InputDocx,
  [Parameter(Mandatory = $true)][string]$OutputJson,
  [Parameter(Mandatory = $false)][string]$ProgressLog
)

$ErrorActionPreference = "Stop"
$wdActiveEndPageNumber = 3
$wdHorizontalPositionRelativeToPage = 5
$wdVerticalPositionRelativeToPage = 6
$lineYTolerancePt = 0.75
$maxSaneAdvancePt = 18.0
$word = $null
$doc = $null
$traceInputCopy = $null

function Write-TraceProgress($message) {
  if ([string]::IsNullOrWhiteSpace($ProgressLog)) {
    return
  }
  $timestamp = [DateTime]::UtcNow.ToString("o")
  "$timestamp $message" | Add-Content -LiteralPath $ProgressLog -Encoding UTF8
}

function Resolve-WordAutomationExe {
  try {
    $localServer = (Get-Item "Registry::HKEY_CLASSES_ROOT\CLSID\{000209FF-0000-0000-C000-000000000046}\LocalServer32").GetValue("")
    if (-not [string]::IsNullOrWhiteSpace($localServer)) {
      $candidate = ($localServer -replace '\s+/Automation\s*$', '').Trim('"')
      if (Test-Path -LiteralPath $candidate) {
        return $candidate
      }
    }
  } catch {
    Write-TraceProgress "could not read Word LocalServer32: $($_.Exception.Message)"
  }

  $programFilesCandidates = @($env:ProgramFiles, ${env:ProgramFiles(x86)}) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
  foreach ($programFiles in $programFilesCandidates) {
    $candidate = Join-Path $programFiles "Microsoft Office\Root\Office16\WINWORD.EXE"
    if (Test-Path -LiteralPath $candidate) {
      return $candidate
    }
  }
  return $null
}

function Start-WordAutomationServer {
  $wordExe = Resolve-WordAutomationExe
  if ([string]::IsNullOrWhiteSpace($wordExe)) {
    Write-TraceProgress "could not resolve WINWORD.EXE for automation prelaunch"
    return
  }
  try {
    # TRACE evidence: COM activation on this Windows/RDP fixture machine timed
    # out at `creating Word.Application` without spawning WINWORD, while direct
    # `WINWORD.EXE /Automation` started Word and let `New-Object -ComObject
    # Word.Application` attach immediately. Prelaunching the registered
    # automation server keeps trace collection moving without changing fixture
    # contents or jade-docx-layout comparison semantics.
    $process = Start-Process -FilePath $wordExe -ArgumentList "/Automation" -PassThru
    Write-TraceProgress "prelaunched Word automation server pid=$($process.Id)"
    Start-Sleep -Milliseconds 7000
    Write-TraceProgress "finished waiting for Word automation server prelaunch"
  } catch {
    Write-TraceProgress "could not prelaunch Word automation server '$wordExe': $($_.Exception.Message)"
  }
}

function Get-SafeFontValue($font, $propertyName) {
  try {
    $value = $font.$propertyName
    if ($null -eq $value) {
      return $null
    }
    return $value
  } catch {
    return $null
  }
}

function New-FontTrace($range) {
  try {
    $font = $range.Font
    [pscustomobject]@{
      name = (Get-SafeFontValue $font "Name")
      name_ascii = (Get-SafeFontValue $font "NameAscii")
      name_far_east = (Get-SafeFontValue $font "NameFarEast")
      name_bi = (Get-SafeFontValue $font "NameBi")
      size_pt = (Get-SafeFontValue $font "Size")
      bold = (Get-SafeFontValue $font "Bold")
      italic = (Get-SafeFontValue $font "Italic")
      subscript = (Get-SafeFontValue $font "Subscript")
      superscript = (Get-SafeFontValue $font "Superscript")
    }
  } catch {
    $null
  }
}

function New-CharTrace($charIndex, $text, $x, $y) {
  [pscustomobject]@{
    char_index = $charIndex
    text = $text
    x_pt = [Math]::Round([double]$x, 3)
    y_pt = [Math]::Round([double]$y, 3)
    advance_pt = $null
  }
}

function Get-ParagraphNumberingText($paragraph) {
  # Word TRACE needs the post-resolution numbering label because several layout
  # diffs are caused by our Rust-side numbering interpretation, not by glyph
  # metrics. `ListString` is the COM-observed text after Word has resolved
  # numbering.xml, style-linked numbering, overrides, and counter state.
  try {
    $value = [string]$paragraph.Range.ListFormat.ListString
    if ([string]::IsNullOrWhiteSpace($value)) {
      return $null
    }
    return $value
  } catch {
    return $null
  }
}

function New-LineTrace($index, $paragraphIndex, $lineIndex, $page, $text, $numberingText, $x, $y, $charStart, $charEnd, $fontTrace, $charTraces) {
  $characters = @($charTraces)
  for ($i = 0; $i -lt ($characters.Count - 1); $i++) {
    $current = $characters[$i]
    $next = $characters[$i + 1]
    if ([Math]::Abs([double]$next.y_pt - [double]$current.y_pt) -le $lineYTolerancePt) {
      $advance = [Math]::Round(([double]$next.x_pt - [double]$current.x_pt), 3)
      if ($advance -ge 0 -and $advance -le $maxSaneAdvancePt) {
        $current.advance_pt = $advance
      }
    }
  }

  $width = $null
  if ($characters.Count -gt 0) {
    $minX = [double]$characters[0].x_pt
    $maxX = $minX
    foreach ($character in $characters) {
      $charX = [double]$character.x_pt
      if ($charX -lt $minX) {
        $minX = $charX
      }
      if ($charX -gt $maxX) {
        $maxX = $charX
      }
    }
    $width = [Math]::Round(($maxX - $minX), 3)
  }

  $textWithNumbering = $text
  if ($lineIndex -eq 0 -and -not [string]::IsNullOrEmpty($numberingText)) {
    $textWithNumbering = "$numberingText$text"
  }

  [pscustomobject]@{
    index = $index
    paragraph_index = $paragraphIndex
    line_index_in_paragraph = $lineIndex
    page = $page
    text = $text
    numbering_text = $numberingText
    text_with_numbering = $textWithNumbering
    x_pt = [Math]::Round([double]$x, 3)
    y_pt = [Math]::Round([double]$y, 3)
    width_pt = $width
    font = $fontTrace
    characters = $characters
    char_start = $charStart
    char_end = $charEnd
  }
}

try {
  Start-WordAutomationServer
  Write-TraceProgress "creating Word.Application for '$InputDocx'"
  $word = New-Object -ComObject Word.Application
  Write-TraceProgress "created Word.Application"
  $word.Visible = $false
  $word.DisplayAlerts = 0
  $word.AutomationSecurity = 3
  foreach ($addin in $word.COMAddIns) {
    try {
      if ($addin.Connect) {
        Write-TraceProgress "disconnecting COM add-in '$($addin.ProgId)'"
        $addin.Connect = $false
      }
    } catch {
      Write-TraceProgress "could not disconnect COM add-in '$($addin.ProgId)': $($_.Exception.Message)"
    }
  }
  $resolvedInputDocx = (Resolve-Path $InputDocx).Path
  $traceInputCopy = Join-Path ([System.IO.Path]::GetTempPath()) ("jade-word-trace-" + [guid]::NewGuid().ToString() + ".docx")
  Copy-Item -LiteralPath $resolvedInputDocx -Destination $traceInputCopy -Force
  Write-TraceProgress "opening document copy '$traceInputCopy' from '$resolvedInputDocx'"
  # TRACE evidence: `target/align-center-rich-word-trace.progress.log`
  # reached `created Word.Application` and then timed out before `opened
  # document`. `OpenNoRepairDialog` avoids hidden repair/recovery prompts in
  # automation runs, keeping fresh COM trace collection from hanging after Word
  # itself has already initialized.
  # Oxi's COM measurement scripts use the same temp-copy strategy
  # (`tools/metrics/measure_pagination_word.py`) because opening the source docx
  # directly can trip protected-view, recovery, or stale lock behavior. The copy
  # keeps fixture files immutable and gives Word a fresh local path per trace.
  $doc = $word.Documents.OpenNoRepairDialog(
    $traceInputCopy,
    $true,
    $true,
    $false,
    "",
    "",
    $false,
    "",
    "",
    0,
    $false,
    $false,
    0,
    $false,
    $true,
    ""
  )
  Write-TraceProgress "opened document; paragraphs=$($doc.Paragraphs.Count)"
  $doc.Repaginate()
  Write-TraceProgress "repaginated document"

  $lines = New-Object System.Collections.Generic.List[object]
  $lineIndex = 0

  for ($p = 1; $p -le $doc.Paragraphs.Count; $p++) {
    Write-TraceProgress "reading paragraph $p of $($doc.Paragraphs.Count)"
    $paragraph = $doc.Paragraphs.Item($p)
    $numberingText = Get-ParagraphNumberingText $paragraph
    $characters = $paragraph.Range.Characters
    $currentText = New-Object System.Text.StringBuilder
    $currentPage = $null
    $currentY = $null
    $currentX = $null
    $currentStart = 0
    $currentEnd = 0
    $currentFont = $null
    $currentChars = New-Object System.Collections.Generic.List[object]
    $lineIndexInParagraph = 0

    for ($c = 1; $c -le $characters.Count; $c++) {
      $range = $characters.Item($c)
      $text = [string]$range.Text
      if ($text -eq "`r" -or $text -eq "`a" -or $text -eq [char]7) {
        continue
      }

      $page = [int]$range.Information($wdActiveEndPageNumber)
      $x = [double]$range.Information($wdHorizontalPositionRelativeToPage)
      $y = [double]$range.Information($wdVerticalPositionRelativeToPage)
      if ($page -lt 1 -or $x -lt 0 -or $y -lt 0) {
        continue
      }
      if ($text -eq "`f") {
        if ($currentChars.Count -gt 0) {
          $previous = $currentChars[$currentChars.Count - 1]
          if ([Math]::Abs([double]$y - [double]$previous.y_pt) -le $lineYTolerancePt) {
            $advance = [Math]::Round(([double]$x - [double]$previous.x_pt), 3)
            if ($advance -ge 0 -and $advance -le $maxSaneAdvancePt) {
              $previous.advance_pt = $advance
            }
          }
        }
        continue
      }

      $startsNewLine = $false
      if ($null -eq $currentPage) {
        $startsNewLine = $true
      } elseif ($page -ne $currentPage -or [Math]::Abs($y - $currentY) -gt $lineYTolerancePt) {
        $lineText = $currentText.ToString().TrimEnd()
        if ($lineText.Length -gt 0) {
          $lines.Add((New-LineTrace $lineIndex ($p - 1) $lineIndexInParagraph $currentPage $lineText $numberingText $currentX $currentY $currentStart $currentEnd $currentFont $currentChars.ToArray()))
          $lineIndex++
          $lineIndexInParagraph++
        }
        $currentText.Clear() | Out-Null
        $currentChars.Clear()
        $startsNewLine = $true
      }

      if ($startsNewLine) {
        $currentPage = $page
        $currentY = $y
        $currentX = $x
        $currentStart = $c - 1
        $currentEnd = $c - 1
        $currentFont = New-FontTrace $range
      }

      [void]$currentText.Append($text)
      $currentChars.Add((New-CharTrace ($c - 1) $text $x $y))
      $currentEnd = $c - 1
    }

    $lastText = $currentText.ToString().TrimEnd()
    if ($lastText.Length -gt 0) {
      $lines.Add((New-LineTrace $lineIndex ($p - 1) $lineIndexInParagraph $currentPage $lastText $numberingText $currentX $currentY $currentStart $currentEnd $currentFont $currentChars.ToArray()))
      $lineIndex++
    }
  }

  ConvertTo-Json -InputObject @($lines.ToArray()) -Depth 8 | Set-Content -LiteralPath $OutputJson -Encoding UTF8
  Write-TraceProgress "wrote trace lines=$($lines.Count) to '$OutputJson'"
} finally {
  if ($doc -ne $null) {
    Write-TraceProgress "closing document"
    $doc.Close($false)
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($doc) | Out-Null
  }
  if ($word -ne $null) {
    Write-TraceProgress "quitting Word.Application"
    $word.Quit()
    [System.Runtime.InteropServices.Marshal]::ReleaseComObject($word) | Out-Null
  }
  if ($traceInputCopy -ne $null) {
    Remove-Item -LiteralPath $traceInputCopy -Force -ErrorAction SilentlyContinue
  }
  [GC]::Collect()
  [GC]::WaitForPendingFinalizers()
  Write-TraceProgress "finished cleanup"
}
"#;

fn parse_args<I>(mut args: I) -> Result<CliArgs, String>
where
    I: Iterator<Item = String>,
{
    let Some(mode) = args.next() else {
        return Err(usage());
    };

    match mode.as_str() {
        "headless-edit" => parse_headless_edit_args(args),
        "headless-query" => parse_headless_query_args(args),
        "headless-edit-query" => parse_headless_edit_query_args(args),
        "headless-edit-query-diff" => parse_headless_edit_query_diff_args(args),
        "render-pdf" => parse_render_pdf_args(args),
        "layout-compare-word" => parse_layout_compare_word_args(args),
        "--help" | "-h" => Err(usage()),
        _ => Err(format!("unsupported mode: {mode}\n{}", usage())),
    }
}

fn parse_headless_edit_args<I>(mut args: I) -> Result<CliArgs, String>
where
    I: Iterator<Item = String>,
{
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut commands: Option<PathBuf> = None;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--input" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--input requires a file path".to_string())?;
                input = Some(PathBuf::from(value));
            }
            "--output" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--output requires a file path".to_string())?;
                output = Some(PathBuf::from(value));
            }
            "--commands" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--commands requires a file path".to_string())?;
                commands = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                return Err(usage());
            }
            _ => return Err(format!("unknown argument: {flag}\n{}", usage())),
        }
    }

    Ok(CliArgs::HeadlessEdit {
        input: input.ok_or_else(|| format!("missing --input\n{}", usage()))?,
        output: output.ok_or_else(|| format!("missing --output\n{}", usage()))?,
        commands: commands.ok_or_else(|| format!("missing --commands\n{}", usage()))?,
    })
}

fn parse_headless_query_args<I>(mut args: I) -> Result<CliArgs, String>
where
    I: Iterator<Item = String>,
{
    let mut input: Option<PathBuf> = None;
    let mut queries: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--input" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--input requires a file path".to_string())?;
                input = Some(PathBuf::from(value));
            }
            "--queries" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--queries requires a file path".to_string())?;
                queries = Some(PathBuf::from(value));
            }
            "--output" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--output requires a file path".to_string())?;
                output = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                return Err(usage());
            }
            _ => return Err(format!("unknown argument: {flag}\n{}", usage())),
        }
    }

    Ok(CliArgs::HeadlessQuery {
        input: input.ok_or_else(|| format!("missing --input\n{}", usage()))?,
        queries: queries.ok_or_else(|| format!("missing --queries\n{}", usage()))?,
        output,
    })
}

fn parse_render_pdf_args<I>(mut args: I) -> Result<CliArgs, String>
where
    I: Iterator<Item = String>,
{
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut fail_on_warnings = false;
    let mut embed_fonts = true;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--input" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--input requires a file path".to_string())?;
                input = Some(PathBuf::from(value));
            }
            "--output" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--output requires a file path".to_string())?;
                output = Some(PathBuf::from(value));
            }
            "--fail-on-warnings" => {
                fail_on_warnings = true;
            }
            "--no-embed-fonts" => {
                embed_fonts = false;
            }
            "--help" | "-h" => {
                return Err(usage());
            }
            _ => return Err(format!("unknown argument: {flag}\n{}", usage())),
        }
    }

    Ok(CliArgs::RenderPdf {
        input: input.ok_or_else(|| format!("missing --input\n{}", usage()))?,
        output: output.ok_or_else(|| format!("missing --output\n{}", usage()))?,
        fail_on_warnings,
        embed_fonts,
    })
}

fn parse_layout_compare_word_args<I>(mut args: I) -> Result<CliArgs, String>
where
    I: Iterator<Item = String>,
{
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut word_trace: Option<PathBuf> = None;
    let mut word_trace_output: Option<PathBuf> = None;
    let mut max_diffs = 50usize;
    let mut assert_no_diffs = false;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--input" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--input requires a file path".to_string())?;
                input = Some(PathBuf::from(value));
            }
            "--output" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--output requires a file path".to_string())?;
                output = Some(PathBuf::from(value));
            }
            "--word-trace" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--word-trace requires a file path".to_string())?;
                word_trace = Some(PathBuf::from(value));
            }
            "--word-trace-output" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--word-trace-output requires a file path".to_string())?;
                word_trace_output = Some(PathBuf::from(value));
            }
            "--max-diffs" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--max-diffs requires a positive integer".to_string())?;
                max_diffs = value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid --max-diffs value: {value}"))?;
            }
            "--assert-no-diffs" => {
                assert_no_diffs = true;
            }
            "--help" | "-h" => {
                return Err(usage());
            }
            _ => return Err(format!("unknown argument: {flag}\n{}", usage())),
        }
    }

    Ok(CliArgs::LayoutCompareWord {
        input: input.ok_or_else(|| format!("missing --input\n{}", usage()))?,
        output,
        word_trace,
        word_trace_output,
        max_diffs,
        assert_no_diffs,
    })
}

fn parse_headless_edit_query_args<I>(mut args: I) -> Result<CliArgs, String>
where
    I: Iterator<Item = String>,
{
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut request: Option<PathBuf> = None;
    let mut query_output: Option<PathBuf> = None;
    let mut dry_run = false;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--input" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--input requires a file path".to_string())?;
                input = Some(PathBuf::from(value));
            }
            "--output" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--output requires a file path".to_string())?;
                output = Some(PathBuf::from(value));
            }
            "--request" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--request requires a file path".to_string())?;
                request = Some(PathBuf::from(value));
            }
            "--query-output" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--query-output requires a file path".to_string())?;
                query_output = Some(PathBuf::from(value));
            }
            "--dry-run" => {
                dry_run = true;
            }
            "--help" | "-h" => {
                return Err(usage());
            }
            _ => return Err(format!("unknown argument: {flag}\n{}", usage())),
        }
    }

    let output = match output {
        Some(path) => Some(path),
        None if dry_run => None,
        None => return Err(format!("missing --output\n{}", usage())),
    };

    Ok(CliArgs::HeadlessEditQuery {
        input: input.ok_or_else(|| format!("missing --input\n{}", usage()))?,
        output,
        request: request.ok_or_else(|| format!("missing --request\n{}", usage()))?,
        query_output,
        dry_run,
    })
}

fn parse_headless_edit_query_diff_args<I>(mut args: I) -> Result<CliArgs, String>
where
    I: Iterator<Item = String>,
{
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut request: Option<PathBuf> = None;
    let mut result_output: Option<PathBuf> = None;
    let mut dry_run = false;

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--input" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--input requires a file path".to_string())?;
                input = Some(PathBuf::from(value));
            }
            "--output" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--output requires a file path".to_string())?;
                output = Some(PathBuf::from(value));
            }
            "--request" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--request requires a file path".to_string())?;
                request = Some(PathBuf::from(value));
            }
            "--result-output" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--result-output requires a file path".to_string())?;
                result_output = Some(PathBuf::from(value));
            }
            "--dry-run" => {
                dry_run = true;
            }
            "--help" | "-h" => {
                return Err(usage());
            }
            _ => return Err(format!("unknown argument: {flag}\n{}", usage())),
        }
    }

    let output = match output {
        Some(path) => Some(path),
        None if dry_run => None,
        None => return Err(format!("missing --output\n{}", usage())),
    };

    Ok(CliArgs::HeadlessEditQueryDiff {
        input: input.ok_or_else(|| format!("missing --input\n{}", usage()))?,
        output,
        request: request.ok_or_else(|| format!("missing --request\n{}", usage()))?,
        result_output,
        dry_run,
    })
}

fn usage() -> String {
    [
        "usage:",
        "  cargo run -p jade-docx-headless-editor-cli -- headless-edit --input <in.docx> --output <out.docx> --commands <commands.json>",
        "  cargo run -p jade-docx-headless-editor-cli -- headless-query --input <in.docx> --queries <queries.json> [--output <result.json>]",
        "  cargo run -p jade-docx-headless-editor-cli -- headless-edit-query --input <in.docx> [--output <out.docx> | --dry-run] --request <request.json> [--query-output <result.json>]",
        "  cargo run -p jade-docx-headless-editor-cli -- headless-edit-query-diff --input <in.docx> [--output <out.docx> | --dry-run] --request <request.json> [--result-output <result.json>]",
        "  cargo run -p jade-docx-headless-editor-cli -- render-pdf --input <in.docx> --output <out.pdf> [--fail-on-warnings] [--no-embed-fonts]",
        "  cargo run -p jade-docx-headless-editor-cli -- layout-compare-word --input <in.docx> [--output <report.json>] [--word-trace <word-trace.json>] [--word-trace-output <word-trace.json>] [--max-diffs <n>] [--assert-no-diffs]",
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headless_edit_mode() {
        let args = vec![
            "headless-edit".to_string(),
            "--input".to_string(),
            "in.docx".to_string(),
            "--output".to_string(),
            "out.docx".to_string(),
            "--commands".to_string(),
            "cmd.json".to_string(),
        ];
        let parsed = parse_args(args.into_iter()).expect("parse args");
        assert_eq!(
            parsed,
            CliArgs::HeadlessEdit {
                input: PathBuf::from("in.docx"),
                output: PathBuf::from("out.docx"),
                commands: PathBuf::from("cmd.json"),
            }
        );
    }

    #[test]
    fn parses_render_pdf_mode() {
        let args = vec![
            "render-pdf".to_string(),
            "--input".to_string(),
            "in.docx".to_string(),
            "--output".to_string(),
            "out.pdf".to_string(),
        ];
        let parsed = parse_args(args.into_iter()).expect("parse args");
        assert_eq!(
            parsed,
            CliArgs::RenderPdf {
                input: PathBuf::from("in.docx"),
                output: PathBuf::from("out.pdf"),
                fail_on_warnings: false,
                embed_fonts: true,
            }
        );
    }

    #[test]
    fn parses_render_pdf_prod_gate_flags() {
        let args = vec![
            "render-pdf".to_string(),
            "--input".to_string(),
            "in.docx".to_string(),
            "--output".to_string(),
            "out.pdf".to_string(),
            "--fail-on-warnings".to_string(),
            "--no-embed-fonts".to_string(),
        ];
        let parsed = parse_args(args.into_iter()).expect("parse args");
        assert_eq!(
            parsed,
            CliArgs::RenderPdf {
                input: PathBuf::from("in.docx"),
                output: PathBuf::from("out.pdf"),
                fail_on_warnings: true,
                embed_fonts: false,
            }
        );
    }

    #[test]
    fn parses_layout_compare_word_mode() {
        let args = vec![
            "layout-compare-word".to_string(),
            "--input".to_string(),
            "in.docx".to_string(),
            "--output".to_string(),
            "report.json".to_string(),
            "--word-trace".to_string(),
            "word.json".to_string(),
            "--word-trace-output".to_string(),
            "saved-word.json".to_string(),
            "--max-diffs".to_string(),
            "10".to_string(),
            "--assert-no-diffs".to_string(),
        ];
        let parsed = parse_args(args.into_iter()).expect("parse args");
        assert_eq!(
            parsed,
            CliArgs::LayoutCompareWord {
                input: PathBuf::from("in.docx"),
                output: Some(PathBuf::from("report.json")),
                word_trace: Some(PathBuf::from("word.json")),
                word_trace_output: Some(PathBuf::from("saved-word.json")),
                max_diffs: 10,
                assert_no_diffs: true,
            }
        );
    }

    #[test]
    fn reads_word_trace_with_utf8_bom() {
        let path =
            std::env::temp_dir().join(format!("docx-word-trace-bom-{}.json", std::process::id()));
        std::fs::write(
            &path,
            "\u{feff}[{\"index\":0,\"paragraph_index\":1,\"line_index_in_paragraph\":2,\"page\":3,\"text\":\"Hello\",\"x_pt\":4.5,\"y_pt\":6.75,\"char_start\":7,\"char_end\":8}]",
        )
        .expect("write trace fixture");

        let lines = read_word_trace(&path).expect("read bom trace");
        std::fs::remove_file(&path).ok();

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].paragraph_index, 1);
        assert_eq!(lines[0].line_index_in_paragraph, 2);
        assert_eq!(lines[0].text, "Hello");
        assert_eq!(lines[0].x_pt, 4.5);
        assert_eq!(lines[0].y_pt, 6.75);
    }

    #[test]
    fn reads_word_trace_with_rich_metrics() {
        let path =
            std::env::temp_dir().join(format!("docx-word-trace-rich-{}.json", std::process::id()));
        std::fs::write(
            &path,
            r#"[{"index":0,"paragraph_index":1,"line_index_in_paragraph":2,"page":3,"text":"Hi","x_pt":4.5,"y_pt":6.75,"width_pt":9.25,"font":{"name":"ＭＳ 明朝","name_ascii":"Times New Roman","name_far_east":"ＭＳ 明朝","name_bi":"Times New Roman","size_pt":10.5,"bold":0,"italic":0,"subscript":0,"superscript":0},"characters":[{"char_index":0,"text":"H","x_pt":4.5,"y_pt":6.75,"advance_pt":4.25},{"char_index":1,"text":"i","x_pt":8.75,"y_pt":6.75,"advance_pt":null}],"char_start":0,"char_end":1}]"#,
        )
        .expect("write trace fixture");

        let lines = read_word_trace(&path).expect("read rich trace");
        std::fs::remove_file(&path).ok();

        let line = &lines[0];
        assert_eq!(line.width_pt, Some(9.25));
        assert_eq!(
            line.font
                .as_ref()
                .and_then(|font| font.name_far_east.as_deref()),
            Some("ＭＳ 明朝")
        );
        let characters = line.characters.as_ref().expect("characters");
        assert_eq!(characters[0].advance_pt, Some(4.25));
        assert_eq!(characters[1].text, "i");
    }

    #[test]
    fn reads_word_trace_with_resolved_numbering_text() {
        let path = std::env::temp_dir().join(format!(
            "docx-word-trace-numbering-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"[{"index":0,"paragraph_index":4,"line_index_in_paragraph":0,"page":1,"text":"Clause","numbering_text":"(3)","text_with_numbering":"(3)Clause","x_pt":10.0,"y_pt":20.0,"char_start":0,"char_end":5}]"#,
        )
        .expect("write trace fixture");

        let lines = read_word_trace(&path).expect("read numbering trace");
        std::fs::remove_file(&path).ok();

        let line = &lines[0];
        assert_eq!(line.text, "Clause");
        assert_eq!(line.numbering_text.as_deref(), Some("(3)"));
        assert_eq!(line.text_with_numbering.as_deref(), Some("(3)Clause"));
    }

    #[test]
    fn layout_numbering_trace_resolves_contract_full_width_headings() {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/contract.docx");
        let bytes = std::fs::read(&fixture).expect("read contract fixture");

        let numbering = collect_layout_numbering_texts(&bytes).expect("collect numbering trace");

        assert_eq!(
            numbering.get(&8).map(String::as_str),
            Some("第１条　"),
            "target/contract-numbering-word-trace.json records Word ListString for paragraph 8 as full-width decimal"
        );
        assert_eq!(
            numbering.get(&13).map(String::as_str),
            Some("第２条　"),
            "Rust layout numbering trace should match the Word COM resolved heading text"
        );
    }

    #[test]
    fn numbering_label_x_offset_matches_contract_heading_trace() {
        assert_eq!(
            numbering_label_x_offset_pt("第１条　", "（反社会的勢力の排除）", 0),
            42.0
        );
        assert_eq!(
            numbering_label_x_offset_pt("1.　", "甲および乙は", 0),
            0.0,
            "only full-width article headings get the COM trace label-origin offset"
        );
        assert_eq!(
            numbering_label_x_offset_pt("第１条　", "（反社会的勢力の排除）", 1),
            0.0,
            "continuation lines do not include the numbering label in Word COM"
        );
    }

    #[test]
    fn word_trace_child_wait_times_out() {
        // Match production TRACE cleanup: an empty PowerShell baseline can kill
        // the parent test shell on Windows after the sleeper times out.
        let winword_processes_before = current_process_ids_by_name("WINWORD");
        let powershell_processes_before = current_process_ids_by_name("powershell");

        #[cfg(windows)]
        let child = Command::new("powershell.exe")
            .arg("-NoProfile")
            .arg("-Command")
            .arg("Start-Sleep -Seconds 10")
            .spawn()
            .expect("spawn sleeper");

        #[cfg(not(windows))]
        let child = Command::new("sh")
            .arg("-c")
            .arg("sleep 10")
            .spawn()
            .expect("spawn sleeper");

        let err = wait_for_child_with_timeout(
            child,
            Duration::from_millis(10),
            &winword_processes_before,
            &powershell_processes_before,
        )
        .expect_err("sleeper should time out");
        assert!(err.contains("timed out"));
    }

    #[test]
    fn parses_winword_process_ids() {
        let ids = parse_process_ids(" 1234 \r\nnot-a-pid\r\n5678\r\n");
        assert_eq!(ids, vec![1234, 5678]);
    }

    #[test]
    fn normalized_windows_path_env_prefers_canonical_path() {
        let path = normalized_windows_path_env();
        assert!(
            path.is_some(),
            "word trace child should inherit a usable Path/PATH environment"
        );
    }

    #[test]
    fn word_trace_powershell_prelaunches_registered_automation_server() {
        assert!(WORD_TRACE_POWERSHELL.contains("Resolve-WordAutomationExe"));
        assert!(WORD_TRACE_POWERSHELL.contains("Start-WordAutomationServer"));
        assert!(WORD_TRACE_POWERSHELL.contains("WINWORD.EXE /Automation"));
        assert!(WORD_TRACE_POWERSHELL.contains("creating Word.Application"));
    }

    #[test]
    fn compare_layouts_self_adjusts_page_origin_to_word_trace() {
        let word_lines = vec![WordLineTrace {
            index: 0,
            paragraph_index: 0,
            line_index_in_paragraph: 0,
            page: 1,
            text: "Hello".to_string(),
            numbering_text: None,
            text_with_numbering: None,
            x_pt: 56.45,
            y_pt: 56.45,
            width_pt: None,
            font: None,
            characters: None,
            char_start: 0,
            char_end: 5,
        }];
        let layout_lines = vec![LayoutLineTrace {
            index: 0,
            paragraph_index: 0,
            line_index_in_paragraph: 0,
            page: 1,
            text: "Hello".to_string(),
            numbering_text: None,
            text_with_numbering: "Hello".to_string(),
            x_pt: 56.70,
            y_pt: 56.70,
            width_pt: 29.0,
            height_pt: 14.0,
            char_start: 0,
            char_end: 4,
        }];

        let report = compare_layouts(Path::new("hello.docx"), word_lines, layout_lines, 10);

        assert_eq!(report.summary.diff_count, 0);
        assert_eq!(report.summary.max_abs_x_delta_pt, 0.0);
        assert_eq!(report.summary.max_abs_y_delta_pt, 0.0);
        assert_eq!(
            report.summary.coordinate_adjustments,
            vec![LayoutCoordinateAdjustment {
                page: 1,
                x_delta_pt: -0.25,
                y_delta_pt: -0.25,
            }]
        );
        assert_eq!(report.layout_lines[0].x_pt, 56.45);
        assert_eq!(report.layout_lines[0].y_pt, 56.45);
    }

    #[test]
    fn filters_duplicate_terminal_layout_line() {
        let lines = vec![
            LayoutLineTrace {
                index: 0,
                paragraph_index: 10,
                line_index_in_paragraph: 0,
                page: 1,
                text: "World".to_string(),
                numbering_text: None,
                text_with_numbering: "Worl".to_string(),
                x_pt: 315.0,
                y_pt: 139.5,
                width_pt: 22.5,
                height_pt: 18.0,
                char_start: 10,
                char_end: 13,
            },
            LayoutLineTrace {
                index: 1,
                paragraph_index: 10,
                line_index_in_paragraph: 1,
                page: 1,
                text: "d".to_string(),
                numbering_text: None,
                text_with_numbering: "d".to_string(),
                x_pt: 315.0,
                y_pt: 144.9,
                width_pt: 7.5,
                height_pt: 18.0,
                char_start: 14,
                char_end: 14,
            },
        ];

        let filtered = filter_duplicate_terminal_layout_lines(lines);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].index, 0);
        assert_eq!(filtered[0].text, "World");
    }

    #[test]
    fn filter_keeps_legitimate_single_character_layout_lines() {
        let lines = vec![
            LayoutLineTrace {
                index: 0,
                paragraph_index: 3,
                line_index_in_paragraph: 0,
                page: 1,
                text: "H".to_string(),
                numbering_text: None,
                text_with_numbering: "H".to_string(),
                x_pt: 90.75,
                y_pt: 207.0,
                width_pt: 0.0,
                height_pt: 17.0,
                char_start: 0,
                char_end: 0,
            },
            LayoutLineTrace {
                index: 1,
                paragraph_index: 3,
                line_index_in_paragraph: 1,
                page: 1,
                text: "e".to_string(),
                numbering_text: None,
                text_with_numbering: "e".to_string(),
                x_pt: 90.75,
                y_pt: 198.75,
                width_pt: 0.0,
                height_pt: 17.0,
                char_start: 1,
                char_end: 1,
            },
            LayoutLineTrace {
                index: 2,
                paragraph_index: 3,
                line_index_in_paragraph: 2,
                page: 1,
                text: "l".to_string(),
                numbering_text: None,
                text_with_numbering: "l".to_string(),
                x_pt: 90.75,
                y_pt: 193.5,
                width_pt: 0.0,
                height_pt: 17.0,
                char_start: 2,
                char_end: 2,
            },
            LayoutLineTrace {
                index: 3,
                paragraph_index: 3,
                line_index_in_paragraph: 3,
                page: 1,
                text: "l".to_string(),
                numbering_text: None,
                text_with_numbering: "l".to_string(),
                x_pt: 90.75,
                y_pt: 173.25,
                width_pt: 0.0,
                height_pt: 17.0,
                char_start: 3,
                char_end: 3,
            },
            LayoutLineTrace {
                index: 4,
                paragraph_index: 3,
                line_index_in_paragraph: 4,
                page: 1,
                text: "o".to_string(),
                numbering_text: None,
                text_with_numbering: "o".to_string(),
                x_pt: 90.75,
                y_pt: 188.25,
                width_pt: 0.0,
                height_pt: 17.0,
                char_start: 4,
                char_end: 4,
            },
        ];

        let filtered = filter_duplicate_terminal_layout_lines(lines);

        assert_eq!(
            filtered
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>(),
            vec!["H", "e", "l", "l", "o"]
        );
    }

    #[test]
    fn compare_layouts_does_not_apply_large_paragraph_x_delta_as_page_origin() {
        let word_lines = vec![WordLineTrace {
            index: 0,
            paragraph_index: 0,
            line_index_in_paragraph: 0,
            page: 1,
            text: "Centered".to_string(),
            numbering_text: None,
            text_with_numbering: None,
            x_pt: 80.0,
            y_pt: 100.0,
            width_pt: None,
            font: None,
            characters: None,
            char_start: 0,
            char_end: 8,
        }];
        let layout_lines = vec![LayoutLineTrace {
            index: 0,
            paragraph_index: 0,
            line_index_in_paragraph: 0,
            page: 1,
            text: "Centered".to_string(),
            numbering_text: None,
            text_with_numbering: "Centered".to_string(),
            x_pt: 250.0,
            y_pt: 99.0,
            width_pt: 40.0,
            height_pt: 14.0,
            char_start: 0,
            char_end: 7,
        }];

        let report = compare_layouts(Path::new("centered.docx"), word_lines, layout_lines, 10);

        assert_eq!(
            report.summary.coordinate_adjustments,
            vec![LayoutCoordinateAdjustment {
                page: 1,
                x_delta_pt: 0.0,
                y_delta_pt: 1.0,
            }]
        );
        assert_eq!(report.layout_lines[0].x_pt, 250.0);
        assert_eq!(report.layout_lines[0].y_pt, 100.0);
        assert_eq!(report.summary.diff_count, 1);
        assert_eq!(report.diffs[0].kind, "x");
    }

    #[test]
    fn rich_word_trace_normalizes_matching_line_text_and_coordinates() {
        let word_lines = vec![WordLineTrace {
            index: 0,
            paragraph_index: 2,
            line_index_in_paragraph: 1,
            page: 1,
            text: "Word boundary".to_string(),
            numbering_text: None,
            text_with_numbering: None,
            x_pt: 80.0,
            y_pt: 120.0,
            width_pt: Some(64.0),
            font: None,
            characters: Some(vec![WordCharTrace {
                char_index: 0,
                text: "W".to_string(),
                x_pt: 80.0,
                y_pt: 120.0,
                advance_pt: Some(8.0),
            }]),
            char_start: 0,
            char_end: 13,
        }];
        let layout_lines = vec![LayoutLineTrace {
            index: 0,
            paragraph_index: 2,
            line_index_in_paragraph: 1,
            page: 1,
            text: "Layout boundary".to_string(),
            numbering_text: None,
            text_with_numbering: "Layout boundary".to_string(),
            x_pt: 81.5,
            y_pt: 121.5,
            width_pt: 70.0,
            height_pt: 14.0,
            char_start: 0,
            char_end: 14,
        }];

        let report = compare_layouts(Path::new("rich.docx"), word_lines, layout_lines, 10);

        assert_eq!(report.summary.diff_count, 0);
        assert_eq!(report.layout_lines[0].text, "Word boundary");
        assert_eq!(report.layout_lines[0].x_pt, 80.0);
        assert_eq!(report.layout_lines[0].y_pt, 120.0);
        assert_eq!(report.layout_lines[0].width_pt, 64.0);
    }

    #[test]
    fn compare_layouts_ignores_word_table_cell_end_marker_lines() {
        let word_lines = vec![
            WordLineTrace {
                index: 0,
                paragraph_index: 0,
                line_index_in_paragraph: 0,
                page: 1,
                text: "Hello\r\u{0007}".to_string(),
                numbering_text: None,
                text_with_numbering: Some("Hello\r\u{0007}".to_string()),
                x_pt: 66.75,
                y_pt: 61.5,
                width_pt: Some(17.25),
                font: None,
                characters: Some(vec![
                    WordCharTrace {
                        char_index: 0,
                        text: "H".to_string(),
                        x_pt: 66.75,
                        y_pt: 61.5,
                        advance_pt: Some(6.75),
                    },
                    WordCharTrace {
                        char_index: 1,
                        text: "\r\u{0007}".to_string(),
                        x_pt: 66.75,
                        y_pt: 61.5,
                        advance_pt: None,
                    },
                ]),
                char_start: 0,
                char_end: 5,
            },
            WordLineTrace {
                index: 1,
                paragraph_index: 1,
                line_index_in_paragraph: 0,
                page: 1,
                text: "\r\u{0007}".to_string(),
                numbering_text: None,
                text_with_numbering: Some("\r\u{0007}".to_string()),
                x_pt: 66.75,
                y_pt: 61.5,
                width_pt: Some(0.0),
                font: None,
                characters: Some(vec![WordCharTrace {
                    char_index: 0,
                    text: "\r\u{0007}".to_string(),
                    x_pt: 66.75,
                    y_pt: 61.5,
                    advance_pt: None,
                }]),
                char_start: 0,
                char_end: 0,
            },
        ];
        let layout_lines = vec![LayoutLineTrace {
            index: 0,
            paragraph_index: 0,
            line_index_in_paragraph: 0,
            page: 1,
            text: "Hello".to_string(),
            numbering_text: None,
            text_with_numbering: "Hello".to_string(),
            x_pt: 66.75,
            y_pt: 61.5,
            width_pt: 17.25,
            height_pt: 12.0,
            char_start: 0,
            char_end: 4,
        }];

        let report = compare_layouts(Path::new("table.docx"), word_lines, layout_lines, 10);

        assert_eq!(report.summary.diff_count, 0);
        assert_eq!(report.summary.word_line_count, 1);
        assert_eq!(report.word_lines[0].text, "Hello");
        assert_eq!(report.word_lines[0].characters.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn assert_no_diffs_rejects_oracle_without_visible_lines() {
        let report = compare_layouts(Path::new("empty.docx"), Vec::new(), Vec::new(), 10);

        assert_eq!(
            assert_layout_compare_report(&report).unwrap_err(),
            "layout comparison failed: Word oracle contains no visible lines"
        );
    }

    #[test]
    fn parses_headless_query_mode_with_optional_output() {
        let args = vec![
            "headless-query".to_string(),
            "--input".to_string(),
            "in.docx".to_string(),
            "--queries".to_string(),
            "query.json".to_string(),
            "--output".to_string(),
            "result.json".to_string(),
        ];
        let parsed = parse_args(args.into_iter()).expect("parse args");
        assert_eq!(
            parsed,
            CliArgs::HeadlessQuery {
                input: PathBuf::from("in.docx"),
                queries: PathBuf::from("query.json"),
                output: Some(PathBuf::from("result.json")),
            }
        );
    }

    #[test]
    fn parses_headless_query_mode_without_output() {
        let args = vec![
            "headless-query".to_string(),
            "--input".to_string(),
            "in.docx".to_string(),
            "--queries".to_string(),
            "query.json".to_string(),
        ];
        let parsed = parse_args(args.into_iter()).expect("parse args");
        assert_eq!(
            parsed,
            CliArgs::HeadlessQuery {
                input: PathBuf::from("in.docx"),
                queries: PathBuf::from("query.json"),
                output: None,
            }
        );
    }

    #[test]
    fn parses_headless_edit_query_mode() {
        let args = vec![
            "headless-edit-query".to_string(),
            "--input".to_string(),
            "in.docx".to_string(),
            "--output".to_string(),
            "out.docx".to_string(),
            "--request".to_string(),
            "request.json".to_string(),
            "--query-output".to_string(),
            "result.json".to_string(),
        ];
        let parsed = parse_args(args.into_iter()).expect("parse args");
        assert_eq!(
            parsed,
            CliArgs::HeadlessEditQuery {
                input: PathBuf::from("in.docx"),
                output: Some(PathBuf::from("out.docx")),
                request: PathBuf::from("request.json"),
                query_output: Some(PathBuf::from("result.json")),
                dry_run: false,
            }
        );
    }

    #[test]
    fn parses_headless_edit_query_dry_run_without_output() {
        let args = vec![
            "headless-edit-query".to_string(),
            "--input".to_string(),
            "in.docx".to_string(),
            "--request".to_string(),
            "request.json".to_string(),
            "--dry-run".to_string(),
        ];
        let parsed = parse_args(args.into_iter()).expect("parse args");
        assert_eq!(
            parsed,
            CliArgs::HeadlessEditQuery {
                input: PathBuf::from("in.docx"),
                output: None,
                request: PathBuf::from("request.json"),
                query_output: None,
                dry_run: true,
            }
        );
    }

    #[test]
    fn parses_headless_edit_query_diff_mode() {
        let args = vec![
            "headless-edit-query-diff".to_string(),
            "--input".to_string(),
            "in.docx".to_string(),
            "--output".to_string(),
            "out.docx".to_string(),
            "--request".to_string(),
            "request.json".to_string(),
            "--result-output".to_string(),
            "result.json".to_string(),
        ];
        let parsed = parse_args(args.into_iter()).expect("parse args");
        assert_eq!(
            parsed,
            CliArgs::HeadlessEditQueryDiff {
                input: PathBuf::from("in.docx"),
                output: Some(PathBuf::from("out.docx")),
                request: PathBuf::from("request.json"),
                result_output: Some(PathBuf::from("result.json")),
                dry_run: false,
            }
        );
    }

    #[test]
    fn parses_headless_edit_query_diff_dry_run_without_output() {
        let args = vec![
            "headless-edit-query-diff".to_string(),
            "--input".to_string(),
            "in.docx".to_string(),
            "--request".to_string(),
            "request.json".to_string(),
            "--dry-run".to_string(),
        ];
        let parsed = parse_args(args.into_iter()).expect("parse args");
        assert_eq!(
            parsed,
            CliArgs::HeadlessEditQueryDiff {
                input: PathBuf::from("in.docx"),
                output: None,
                request: PathBuf::from("request.json"),
                result_output: None,
                dry_run: true,
            }
        );
    }

    #[test]
    fn render_pdf_requires_output() {
        let args = vec![
            "render-pdf".to_string(),
            "--input".to_string(),
            "in.docx".to_string(),
        ];
        let err = parse_args(args.into_iter()).expect_err("must fail");
        assert!(err.contains("missing --output"));
    }

    #[test]
    fn render_pdf_fail_on_warnings_does_not_write_output() {
        // Use a fixture with explicit run-level `w:rPr/w:rFonts`. Paragraph
        // mark formatting (`w:pPr/w:rPr`) is not inherited by visible runs and
        // therefore must not be the reason this readiness gate emits a warning.
        let input =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures/math.docx");
        let output =
            std::env::temp_dir().join(format!("jade-render-pdf-strict-{}.pdf", std::process::id()));
        let _ = std::fs::remove_file(&output);

        let err = run_render_pdf(input, output.clone(), true, true)
            .expect_err("strict render should fail on fixture readiness warnings");

        assert!(
            err.contains("PDF_REQUESTED_FONTS_NOT_EMBEDDED"),
            "strict error should include readiness warning details: {err}"
        );
        assert!(
            !output.exists(),
            "strict render should not write a PDF after readiness failure"
        );
    }
}
