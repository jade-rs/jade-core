//! Section vertical alignment "Justified" (`w:vAlign="both"`, ECMA-376 Part 1
//! §17.6.22) rendered on a recording painter: a page's leftover height is
//! spread between its top-level paragraphs so the last line reaches the body
//! bottom, without moving any page break.

use super::recording_host::recording_renderer;
use super::section_tests::build_docx_with_body;
use super::*;
use crate::word_document::WordDocument;
use jade_painter::recording::RecordingPainter;

const MARGINS: &str = r#"<w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#;

fn paragraph(text: &str, properties: &str) -> String {
    format!(
        r#"<w:p><w:pPr><w:spacing w:before="0" w:after="0"/>{properties}</w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>"#
    )
}

/// Section 1 (`first_align`): three short paragraphs. Section 2 (top) starts
/// on a new page with one paragraph, so page 1 ends with a section page break.
fn document(first_align: &str, first_text: &str) -> Document {
    let mut body = String::new();
    body.push_str(&paragraph(first_text, ""));
    body.push_str(&paragraph("Second", ""));
    body.push_str(&paragraph(
        "Third",
        &format!(r#"<w:sectPr>{MARGINS}<w:vAlign w:val="{first_align}"/></w:sectPr>"#),
    ));
    body.push_str(&paragraph("Next page", ""));
    body.push_str(&format!(r#"<w:sectPr>{MARGINS}</w:sectPr>"#));
    let data = build_docx_with_body(&body);
    let word_doc =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let document = word_doc
        .document
        .as_ref()
        .expect("document")
        .as_ref()
        .clone();
    assert_eq!(document.sections.len(), 2, "two explicit section ranges");
    assert_eq!(
        resolve_section_vertical_align(
            document.sections[0]
                .properties
                .as_ref()
                .and_then(|properties| properties.vertical_alignment.as_deref())
        ),
        resolve_section_vertical_align(Some(first_align))
    );
    document
}

/// `(top, bottom)` of the first line of each of the first `count` paragraphs.
fn line_extents(renderer: &CanvasRenderer<RecordingPainter>, count: usize) -> Vec<(f64, f64)> {
    renderer.selection.paragraphs()[..count]
        .iter()
        .map(|paragraph| {
            let line = paragraph.lines.first().expect("line");
            (line.top, line.top + line.height)
        })
        .collect()
}

fn body_bottom(renderer: &CanvasRenderer<RecordingPainter>) -> f64 {
    renderer.page_height - renderer.margin_bottom
}

#[test]
fn justified_page_spreads_paragraphs_to_the_bottom_margin() {
    let mut top_renderer = recording_renderer();
    top_renderer.mark_full_repaint();
    top_renderer
        .render(&document("top", "First"), &Options::default())
        .expect("top render");
    let natural = line_extents(&top_renderer, 3);

    let mut renderer = recording_renderer();
    renderer.mark_full_repaint();
    renderer
        .render(&document("both", "First"), &Options::default())
        .expect("justified render");
    let justified = line_extents(&renderer, 3);

    assert!(
        (justified[0].0 - natural[0].0).abs() < 0.01,
        "the first line stays at the top margin: {justified:?} vs {natural:?}"
    );
    let bottom = body_bottom(&renderer);
    assert!(
        (justified[2].1 - bottom).abs() <= 1.0,
        "the last line ends at the body bottom {bottom}: {justified:?}"
    );
    let gaps: Vec<f64> = (1..3)
        .map(|index| justified[index].0 - justified[index - 1].1)
        .collect();
    let natural_gaps: Vec<f64> = (1..3)
        .map(|index| natural[index].0 - natural[index - 1].1)
        .collect();
    assert!(gaps[0] > 100.0, "leftover is spread: {gaps:?}");
    assert!(
        ((gaps[0] - natural_gaps[0]) - (gaps[1] - natural_gaps[1])).abs() < 0.01,
        "the added gaps are equal: {gaps:?} (natural {natural_gaps:?})"
    );
    assert_eq!(
        renderer.current_page_number, top_renderer.current_page_number,
        "justification never changes pagination"
    );
    assert_eq!(renderer.current_page_number, 2);
    let next = &renderer.selection.paragraphs()[3].lines[0];
    let natural_next = &top_renderer.selection.paragraphs()[3].lines[0];
    assert!(
        (next.top - natural_next.top).abs() < 0.01,
        "the top-aligned next section is untouched"
    );
}

/// A layout change re-measures: a first paragraph that wraps to more lines
/// shrinks the leftover, and the last line still ends at the body bottom.
#[test]
fn justified_gaps_are_remeasured_after_an_edit() {
    let mut renderer = recording_renderer();
    renderer.mark_full_repaint();
    renderer
        .render(&document("both", "First"), &Options::default())
        .expect("first render");
    let before = line_extents(&renderer, 3);

    let long = "word ".repeat(60);
    let edited = document("both", long.trim_end());
    renderer.set_incremental_dirty_from(0);
    renderer
        .render(&edited, &Options::default())
        .expect("edited render");
    let first = &renderer.selection.paragraphs()[0];
    assert!(first.lines.len() > 1, "the edited paragraph wraps");
    let after = line_extents(&renderer, 3);
    let bottom = body_bottom(&renderer);
    assert!(
        (after[2].1 - bottom).abs() <= 1.0,
        "the last line still ends at the body bottom {bottom}: {after:?}"
    );
    assert!(
        after[2].0 - after[1].1 < before[2].0 - before[1].1,
        "the gap shrank with the leftover: {before:?} -> {after:?}"
    );

    let mut fresh = recording_renderer();
    fresh.mark_full_repaint();
    fresh.render(&edited, &Options::default()).expect("fresh");
    let reference = line_extents(&fresh, 3);
    for (edited, fresh) in after.iter().zip(&reference) {
        assert!(
            (edited.0 - fresh.0).abs() < 0.01,
            "the edited layout matches a fresh render: {after:?} vs {reference:?}"
        );
    }
}

/// One paragraph on a justified page has nothing to spread between.
#[test]
fn justified_single_paragraph_page_is_unchanged() {
    let render = |align: &str| {
        let body = format!(
            "{}{}<w:sectPr>{MARGINS}<w:vAlign w:val=\"{align}\"/></w:sectPr>",
            paragraph("Only", ""),
            paragraph("", r#"<w:pageBreakBefore/>"#),
        );
        let data = build_docx_with_body(&body);
        let word_doc = futures::executor::block_on(WordDocument::load(&data, &Options::default()))
            .expect("load");
        let mut renderer = recording_renderer();
        renderer.mark_full_repaint();
        renderer
            .render(
                word_doc.document.as_ref().expect("document"),
                &Options::default(),
            )
            .expect("render");
        line_extents(&renderer, 1)
    };
    assert_eq!(render("both"), render("top"));
}

#[test]
fn justified_gap_splits_leftover_between_slots() {
    assert_eq!(section_justified_gap(900.0, 300.0, 2), 300.0);
    assert_eq!(section_justified_gap(900.0, 300.0, 0), 0.0);
    assert_eq!(section_justified_gap(900.0, 950.0, 3), 0.0);
}

/// The section's final, partly filled page (here the document's last page,
/// ending without any break) is justified too: ECMA-376 §17.6.22 has no
/// last-page exception. Unconfirmed against a Word capture.
#[test]
fn justified_final_page_of_the_document_is_spread() {
    let body = format!(
        "{}{}<w:sectPr>{MARGINS}<w:vAlign w:val=\"both\"/></w:sectPr>",
        paragraph("One", ""),
        paragraph("Two", ""),
    );
    let data = build_docx_with_body(&body);
    let word_doc =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    let mut renderer = recording_renderer();
    renderer.mark_full_repaint();
    renderer
        .render(
            word_doc.document.as_ref().expect("document"),
            &Options::default(),
        )
        .expect("render");
    let lines = line_extents(&renderer, 2);
    assert!(
        (lines[1].1 - body_bottom(&renderer)).abs() <= 1.0,
        "{lines:?}"
    );
}

/// Page count plus, for every paragraph surface, the page of each of its
/// lines: what justification must never change.
type FlowSignature = (usize, Vec<Vec<i64>>);

fn flow_signature(renderer: &CanvasRenderer<RecordingPainter>) -> FlowSignature {
    let lines = renderer
        .selection
        .paragraphs()
        .iter()
        .map(|paragraph| {
            paragraph
                .lines
                .iter()
                .map(|line| (line.absolute_top - line.top).round() as i64)
                .collect()
        })
        .collect();
    (renderer.current_page_number, lines)
}

/// `document` with every section's `w:vAlign` forced to `align`.
fn with_vertical_alignment(document: &Document, align: &str) -> Document {
    let mut document = document.clone();
    let force = |properties: &mut Option<SectionProperties>| {
        properties
            .get_or_insert_with(SectionProperties::default)
            .vertical_alignment = Some(align.to_string());
    };
    force(&mut document.default_section);
    for section in &mut document.sections {
        force(&mut section.properties);
    }
    document
}

/// Outcome of forcing `both` onto one document.
#[derive(Debug, Default)]
struct SweepOutcome {
    /// Justified page count changed the flow (a regression).
    changed: bool,
    /// Pages that received gaps.
    spread_pages: usize,
    /// Justified pages skipped for floats or columns.
    skipped_pages: usize,
    /// Pages whose gaps the painted-pass safety net had to drop.
    fallback_pages: usize,
    /// A render failed or panicked: nothing was compared.
    errored: bool,
    /// A spread page whose lines reach further below its body bottom than
    /// under `top` (page, excess px).
    overflow: Option<(usize, f64)>,
}

/// How far the lowest body line on the page at `offset` reaches below
/// `bottom`. Footnote surfaces are not body lines, and Word for Mac's empty
/// section mark stays out of page vAlign by design (as for `center` and
/// `bottom`), so its line may sit past the bottom. `None` when the page has
/// no body line.
fn page_line_excess(
    renderer: &CanvasRenderer<RecordingPainter>,
    offset: f64,
    bottom: f64,
) -> Option<f64> {
    let excluded = |surface: usize| {
        renderer.valign_section_mark_surfaces.contains(&surface)
            || renderer
                .footnote_surface_ranges
                .iter()
                .any(|(start, end)| (*start..*end).contains(&surface))
    };
    renderer
        .selection
        .paragraphs()
        .iter()
        .enumerate()
        .filter(|(surface, _)| !excluded(*surface))
        .flat_map(|(_, paragraph)| paragraph.lines.iter())
        .filter(|line| ((line.absolute_top - line.top) - offset).abs() < 0.5)
        .map(|line| line.top + line.height - bottom)
        .reduce(f64::max)
}

/// First spread page of `both` whose lines overshoot its real body bottom
/// by more than they do under `top`.
fn spread_page_overflow(
    top: &CanvasRenderer<RecordingPainter>,
    both: &CanvasRenderer<RecordingPainter>,
) -> Option<(usize, f64)> {
    let mut pages: Vec<usize> = both.page_valign_gaps.values().map(|gap| gap.page).collect();
    pages.sort();
    pages.dedup();
    pages.into_iter().find_map(|page| {
        let &(offset, bottom) = both.valign_page_bounds.get(page - 1)?;
        let spread = page_line_excess(both, offset, bottom)?;
        let natural = page_line_excess(top, offset, bottom).unwrap_or(f64::MIN);
        (spread > natural.max(0.0) + 0.5).then_some((page, spread))
    })
}

fn sweep_document(document: &Document) -> SweepOutcome {
    let render = |align: &str| {
        let mut renderer = recording_renderer();
        renderer.mark_full_repaint();
        renderer
            .render(
                &with_vertical_alignment(document, align),
                &Options::default(),
            )
            .map(|_| renderer)
    };
    let (Ok(top), Ok(both)) = (render("top"), render("both")) else {
        return SweepOutcome {
            errored: true,
            ..Default::default()
        };
    };
    let spread: HashSet<usize> = both.page_valign_gaps.values().map(|gap| gap.page).collect();
    SweepOutcome {
        overflow: spread_page_overflow(&top, &both),
        errored: false,
        changed: flow_signature(&top) != flow_signature(&both),
        spread_pages: spread.len(),
        skipped_pages: both.valign_gap_skip_pages.len(),
        fallback_pages: both.valign_flow_fallback_pages.len(),
    }
}

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures")
}

/// Sweeps `names` (fixture file names) on both Word platforms, returning
/// `(platform, name, outcome)` for every rendered document.
fn sweep_fixtures(names: &[String]) -> Vec<(&'static str, String, SweepOutcome)> {
    let threads = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4);
    let chunk = names.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = names
            .chunks(chunk)
            .map(|chunk| {
                scope.spawn(move || {
                    let mut out = Vec::new();
                    for name in chunk {
                        let Ok(data) = std::fs::read(fixture_dir().join(name)) else {
                            continue;
                        };
                        let Ok(word_doc) = futures::executor::block_on(WordDocument::load(
                            &data,
                            &Options::default(),
                        )) else {
                            continue;
                        };
                        let Some(document) = word_doc.document.as_ref() else {
                            continue;
                        };
                        for (label, platform) in [
                            (
                                "windows",
                                jade_docx_layout::line_metrics::WordLayoutPlatform::Windows,
                            ),
                            (
                                "mac",
                                jade_docx_layout::line_metrics::WordLayoutPlatform::Mac,
                            ),
                        ] {
                            jade_docx_layout::line_metrics::set_word_layout_platform(platform);
                            let outcome =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    sweep_document(document)
                                }))
                                .unwrap_or(SweepOutcome {
                                    errored: true,
                                    ..Default::default()
                                });
                            out.push((label, name.clone(), outcome));
                        }
                        jade_docx_layout::line_metrics::set_word_layout_platform(
                            jade_docx_layout::line_metrics::WordLayoutPlatform::Windows,
                        );
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    })
}

fn report(results: &[(&'static str, String, SweepOutcome)]) -> Vec<String> {
    let changed: Vec<String> = results
        .iter()
        .filter(|(_, _, outcome)| outcome.changed)
        .map(|(platform, name, _)| format!("{platform}:{name}"))
        .chain(results.iter().filter_map(|(platform, name, outcome)| {
            let (page, excess) = outcome.overflow?;
            Some(format!(
                "{platform}:{name} overflows page {page} by {excess:.2}px"
            ))
        }))
        .collect();
    let errored: Vec<String> = results
        .iter()
        .filter(|(_, _, outcome)| outcome.errored)
        .map(|(platform, name, _)| format!("{platform}:{name}"))
        .collect();
    let spread = results.iter().filter(|r| r.2.spread_pages > 0).count();
    let skipped: usize = results.iter().map(|r| r.2.skipped_pages).sum();
    let fallback: Vec<String> = results
        .iter()
        .filter(|r| r.2.fallback_pages > 0)
        .map(|(platform, name, outcome)| format!("{platform}:{name}({})", outcome.fallback_pages))
        .collect();
    eprintln!(
        "valign sweep: {} renders, {} errored {:?}, {} spread, {} float/column pages skipped, {} safety-net fallbacks {:?}, {} changed or overflowing",
        results.len(),
        errored.len(),
        errored,
        spread,
        skipped,
        fallback.len(),
        fallback,
        changed.len()
    );
    changed
}

/// Forces `w:vAlign="both"` onto every docx fixture and requires the same
/// pagination as `top`: page count and the page of every line. Slow; run
/// with `cargo test -p jade-docx-editor --lib valign_fixture_sweep -- --ignored`
/// (`JADE_VALIGN_SWEEP_FILTER` narrows the file names).
#[test]
#[ignore]
fn valign_fixture_sweep() {
    let filter = std::env::var("JADE_VALIGN_SWEEP_FILTER").unwrap_or_default();
    let mut names: Vec<String> = std::fs::read_dir(fixture_dir())
        .expect("fixtures")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".docx") && name.contains(&filter))
        .collect();
    names.sort();
    let changed = report(&sweep_fixtures(&names));
    assert!(changed.is_empty(), "flow changed: {changed:#?}");
}

fn load(body: &str) -> Document {
    let data = build_docx_with_body(body);
    let word_doc =
        futures::executor::block_on(WordDocument::load(&data, &Options::default())).expect("load");
    word_doc
        .document
        .as_ref()
        .expect("document")
        .as_ref()
        .clone()
}

fn rendered(document: &Document, options: &Options) -> CanvasRenderer<RecordingPainter> {
    let mut renderer = recording_renderer();
    renderer.mark_full_repaint();
    renderer.render(document, options).expect("render");
    renderer
}

/// Bottom of the last line placed on `page` (1-based).
fn last_line_bottom_on_page(renderer: &CanvasRenderer<RecordingPainter>, page: i64) -> f64 {
    let first_page_offset = renderer
        .selection
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.lines.first())
        .map(|line| (line.absolute_top - line.top).round() as i64)
        .min()
        .expect("lines");
    let offsets: Vec<i64> = {
        let mut offsets: Vec<i64> = renderer
            .selection
            .paragraphs()
            .iter()
            .flat_map(|paragraph| paragraph.lines.iter())
            .map(|line| (line.absolute_top - line.top).round() as i64)
            .collect();
        offsets.sort();
        offsets.dedup();
        offsets
    };
    assert_eq!(offsets[0], first_page_offset);
    let target = offsets[(page - 1) as usize];
    renderer
        .selection
        .paragraphs()
        .iter()
        .flat_map(|paragraph| paragraph.lines.iter())
        .filter(|line| (line.absolute_top - line.top).round() as i64 == target)
        .map(|line| line.top + line.height)
        .fold(f64::MIN, f64::max)
}

/// Collapsed paragraph spacing (before = after = 240) still collapses after
/// the gaps: the shifted pending space-after moves with the text, so a full
/// page stays one page and its last line (not its space-after) reaches the
/// bottom margin.
#[test]
fn justified_spacing_collapse_keeps_pagination_and_drops_final_space_after() {
    let mut body = String::new();
    for index in 0..30 {
        body.push_str(&format!(
            r#"<w:p><w:pPr><w:spacing w:before="240" w:after="240"/></w:pPr><w:r><w:t>Line {index}</w:t></w:r></w:p>"#
        ));
    }
    body.push_str(&format!(r#"<w:sectPr>{MARGINS}</w:sectPr>"#));
    let document = load(&body);
    let top = rendered(
        &with_vertical_alignment(&document, "top"),
        &Options::default(),
    );
    let both = rendered(
        &with_vertical_alignment(&document, "both"),
        &Options::default(),
    );
    assert_eq!(top.current_page_number, 2, "the fixture spans two pages");
    assert_eq!(flow_signature(&both), flow_signature(&top));
    assert!(both.valign_flow_fallback_pages.is_empty());
    let bottom = body_bottom(&both);
    let last = last_line_bottom_on_page(&both, 1);
    assert!(
        (last - bottom).abs() <= 1.0,
        "page 1's last line ends at {last}, body bottom {bottom}"
    );
}

/// Repainting one page from a mid-page (spanning) seek snapshot must not
/// add a resumed element's gap a second time (the snapshot was taken after
/// the gap) nor lose the page's applied shift: the page draws exactly what
/// a full repaint of that page draws.
fn assert_seek_resume_matches_full_repaint(spacing: &str) {
    let long = "word ".repeat(140);
    let mut body = String::new();
    for index in 0..40 {
        body.push_str(&format!(
            r#"<w:p><w:pPr>{spacing}</w:pPr><w:r><w:t>{index} {long}</w:t></w:r></w:p>"#
        ));
    }
    body.push_str(&format!(
        r#"<w:sectPr>{MARGINS}<w:vAlign w:val="both"/></w:sectPr>"#
    ));
    let document = load(&body);
    let mut options = Options {
        render_footnotes: false,
        render_endnotes: false,
        render_changes: false,
        render_comments: false,
        show_formatting_marks: false,
        ..Default::default()
    };
    options.set_pageless(false);
    let painted_text = |renderer: &CanvasRenderer<RecordingPainter>| -> Vec<(String, i64, i64)> {
        let mut glyphs: Vec<_> = renderer
            .host
            .painter()
            .inner()
            .ops()
            .into_iter()
            .filter_map(|op| match op {
                jade_painter::recording::Op::FillText(text, x, y) => {
                    Some((text, (x * 100.0).round() as i64, (y * 100.0).round() as i64))
                }
                _ => None,
            })
            .collect();
        glyphs.sort();
        glyphs
    };
    let mut renderer = rendered(&document, &options);
    assert!(
        !renderer.page_valign_gaps.is_empty(),
        "pages with several paragraphs are spread"
    );
    let mut reference = rendered(&document, &options);
    let pages: Vec<usize> = renderer
        .page_start_seek_cache_by_page
        .borrow()
        .iter()
        .filter(|(_, snapshot)| snapshot.spanning)
        .map(|(page, _)| *page)
        .collect();
    assert!(pages.len() > 3, "spanning snapshots exist");
    let mut resumed = 0;
    for page in pages {
        reference.clear_dirty_regions();
        reference.host.painter().inner().clear();
        reference.set_visible_page_range(Some(page..=page));
        reference.mark_full_repaint();
        reference
            .render(&document, &options)
            .expect("full page render");
        let expected = painted_text(&reference);

        renderer.clear_dirty_regions();
        renderer.host.painter().inner().clear();
        renderer.set_visible_page_range(Some(page..=page));
        renderer.prepare_rerender_for_page(page - 1);
        renderer.render(&document, &options).expect("page render");
        if renderer.last_render_body_element_count < 40 {
            resumed += 1;
        }
        let painted = painted_text(&renderer);
        assert!(!painted.is_empty(), "page {page} painted text");
        assert_eq!(
            painted, expected,
            "repainting page {page} from its seek snapshot ({spacing})"
        );
    }
    assert!(resumed > 0, "at least one repaint resumed from a snapshot");
}

#[test]
fn justified_seek_resume_does_not_shift_twice() {
    assert_seek_resume_matches_full_repaint(r#"<w:spacing w:before="0" w:after="0"/>"#);
}

/// Space-before puts a resumed snapshot below the element's gap position;
/// the page's applied shift must still be adopted.
#[test]
fn justified_seek_resume_with_paragraph_spacing() {
    assert_seek_resume_matches_full_repaint(r#"<w:spacing w:before="240" w:after="240"/>"#);
}

/// A space-after that does not fit is clamped at the page foot: only what
/// it took leaves the used height, so no spread line ends below the body
/// bottom.
#[test]
fn justified_clamped_space_after_never_overflows_the_page() {
    for (after, count) in [(240, 40), (300, 70)] {
        let mut body = String::new();
        for index in 0..count {
            body.push_str(&format!(
                r#"<w:p><w:pPr><w:spacing w:before="0" w:after="{after}"/></w:pPr><w:r><w:t>Line {index}</w:t></w:r></w:p>"#
            ));
        }
        body.push_str(&format!(r#"<w:sectPr>{MARGINS}</w:sectPr>"#));
        let document = load(&body);
        let top = rendered(
            &with_vertical_alignment(&document, "top"),
            &Options::default(),
        );
        let both = rendered(
            &with_vertical_alignment(&document, "both"),
            &Options::default(),
        );
        assert_eq!(flow_signature(&both), flow_signature(&top), "after={after}");
        let spread: HashSet<usize> = both.page_valign_gaps.values().map(|gap| gap.page).collect();
        assert!(!spread.is_empty(), "after={after}: pages are spread");
        for page in spread {
            let (offset, bottom) = both.valign_page_bounds[page - 1];
            let excess = page_line_excess(&both, offset, bottom).expect("body lines");
            assert!(
                excess <= 0.01,
                "after={after}: page {page}'s last line ends {excess}px below the body bottom"
            );
        }
        assert_eq!(spread_page_overflow(&top, &both), None, "after={after}");
    }
}

/// The hook behind the seek fix: an element resumed from a snapshot taken
/// after its gap keeps its position, one resumed from an unshifted snapshot
/// gets the gap once, and both leave the page's applied shift correct.
#[test]
fn justified_gap_is_applied_absolutely_on_resume() {
    let long = "word ".repeat(140);
    let mut body = String::new();
    for index in 0..12 {
        body.push_str(&paragraph(&format!("{index} {long}"), ""));
    }
    body.push_str(&format!(
        r#"<w:sectPr>{MARGINS}<w:vAlign w:val="both"/></w:sectPr>"#
    ));
    let document = load(&body);
    let mut renderer = rendered(&document, &Options::default());
    let (&element, &gap) = renderer
        .page_valign_gaps
        .iter()
        .filter(|(_, gap)| gap.shift > 1.0)
        .min_by_key(|(element, _)| **element)
        .expect("a spread slot");
    let natural = renderer.effective_body_top() + gap.start_used;
    renderer.render_full_repaint = false;
    for (start, label) in [
        (natural + gap.shift, "snapshot taken after the gap"),
        (natural, "unshifted snapshot"),
    ] {
        renderer.current_page_number = gap.page;
        renderer.valign_gap_applied = (0, 0.0);
        renderer.current_y = start;
        renderer.begin_justified_valign_element(element);
        assert!(
            (renderer.current_y - (natural + gap.shift)).abs() < 1e-9,
            "{label}: resumed at {} instead of {}",
            renderer.current_y,
            natural + gap.shift
        );
        assert_eq!(
            renderer.valign_gap_applied,
            (gap.page, gap.shift),
            "{label}"
        );
    }
    // Where a seek snapshot is restored, the hook adopts its position (taken
    // after the gap and any space-before) as is.
    let resumed_at = natural + gap.shift + 16.0;
    renderer.valign_gap_applied = (0, 0.0);
    renderer.valign_seek_resume = true;
    renderer.current_y = resumed_at;
    renderer.begin_justified_valign_element(element);
    assert_eq!(renderer.current_y, resumed_at);
    assert_eq!(renderer.valign_gap_applied, (gap.page, gap.shift));
    assert!(!renderer.valign_seek_resume, "the resume flag is consumed");
}

/// A bounded re-measure (layout page limit) keeps the gaps of the pages
/// past its measured prefix.
#[test]
fn bounded_remeasure_keeps_gaps_past_the_limit() {
    let mut body = String::new();
    for page in 0..3 {
        for index in 0..3 {
            let properties = if page > 0 && index == 0 {
                "<w:pageBreakBefore/>"
            } else {
                ""
            };
            body.push_str(&paragraph(&format!("Page {page} line {index}"), properties));
        }
    }
    body.push_str(&format!(
        r#"<w:sectPr>{MARGINS}<w:vAlign w:val="both"/></w:sectPr>"#
    ));
    let document = load(&body);
    let mut renderer = rendered(&document, &Options::default());
    let before = renderer.page_valign_gaps.clone();
    let spread: HashSet<usize> = before.values().map(|gap| gap.page).collect();
    assert_eq!(spread, HashSet::from([1, 2, 3]));

    renderer.set_layout_page_limit(Some(1));
    renderer.mark_full_repaint();
    renderer
        .render(&document, &Options::default())
        .expect("bounded render");
    assert_eq!(
        renderer.page_valign_gaps, before,
        "pages past the limit keep their gaps"
    );
}

/// The representative fixtures that the pre-fix shift repaginated: tables
/// ending a page, keepNext groups, footnotes, page-break paragraphs, floats,
/// spacing collapse and Mac section marks. Forced to `both`, each must keep
/// the page of every line it has under `top`.
#[test]
fn justified_fixture_subset_keeps_pagination() {
    let names = [
        "word-windows-edge-table-spacing-continuation-compat15.docx",
        "word-windows-edge-table-spacing-at-least-continuation-compat15.docx",
        "word-windows-edge-header-table-left-indent0-ordered-compat15.docx",
        "word-windows-edge-heading-break-paragraph-before480-compat15.docx",
        "word-windows-edge-heading-break-paragraph-direct-page-before-before480-compat14.docx",
        "word-windows-float-lookahead-sides-compat14.docx",
        "word-windows-title-calibri22-sides-spacing0-compat15.docx",
        "word-windows-edge-textbox-arial-size21-compat15.docx",
        "word-mac-probe-float-lookahead.docx",
        "word-mac-probe-grid-line-unit.docx",
        "trust-beneficiary-sale-keep-next.docx",
        "numbering-restart.docx",
        "floating-wrap-sides.docx",
        "floating-inside-outside.docx",
        "floating-square-wrap.docx",
        "footnote-table-textbox-order.docx",
        "footnote-table-each-page.docx",
        "section-break-types.docx",
        "review-kinds.docx",
        "toc.docx",
    ]
    .map(String::from);
    let results = sweep_fixtures(&names);
    assert_eq!(results.len(), names.len() * 2, "every fixture rendered");
    assert!(
        results
            .iter()
            .any(|(_, _, outcome)| outcome.spread_pages > 0),
        "the subset exercises spread pages"
    );
    let changed = report(&results);
    assert!(changed.is_empty(), "flow changed: {changed:#?}");
}

/// Typing cost in a ~50-page document, `both` against `top`: a justified
/// document re-measures on every edit (as `center`/`bottom` do). Prints the
/// mean time per keystroke; run with `--ignored --nocapture`.
#[test]
#[ignore]
fn valign_typing_timing() {
    let mut body = String::new();
    for index in 0..900 {
        body.push_str(&paragraph(
            &format!("{index} {}", "typing text ".repeat(9)),
            "",
        ));
    }
    body.push_str(&format!(r#"<w:sectPr>{MARGINS}</w:sectPr>"#));
    let base = load(&body);
    for align in ["top", "both"] {
        let mut document = with_vertical_alignment(&base, align);
        let mut renderer = rendered(&document, &Options::default());
        let pages = renderer.current_page_number;
        let edited = 450;
        renderer.clear_dirty_regions();
        renderer.set_visible_page_range(Some(pages / 2..=pages / 2));
        let keystrokes = 20;
        let started = std::time::Instant::now();
        for _ in 0..keystrokes {
            if let Some(body) = document.body.as_mut() {
                if let BodyElement::Paragraph(paragraph) = &mut body.elements[edited] {
                    paragraph.runs[0].text.push('x');
                }
            }
            renderer.set_incremental_dirty_from(edited);
            renderer
                .render(&document, &Options::default())
                .expect("edit render");
            renderer.clear_dirty_regions();
        }
        let per_key = started.elapsed() / keystrokes;
        eprintln!(
            "valign typing: {align}: {pages} pages, {per_key:?} per keystroke, last pass laid out {} elements",
            renderer.last_render_body_element_count
        );
    }
}

/// A justified page ending after a paragraph replayed from a cache: the
/// cached layouts record what each space-after really took (the one
/// clamped at the page foot less than its nominal amount), which the replay
/// paths restore, so a re-measure that replays the page's last paragraph
/// sees the same used height as a live layout.
#[test]
fn justified_cached_layouts_keep_the_consumed_space_after() {
    let mut body = String::new();
    for index in 0..40 {
        body.push_str(&format!(
            r#"<w:p><w:pPr><w:spacing w:before="0" w:after="240"/></w:pPr><w:r><w:t>Line {index}</w:t></w:r></w:p>"#
        ));
    }
    body.push_str(&format!(
        r#"<w:sectPr>{MARGINS}<w:vAlign w:val="both"/></w:sectPr>"#
    ));
    let document = load(&body);
    let mut renderer = rendered(&document, &Options::default());
    let mut clamped = 0;
    {
        let cache = renderer.paragraph_layout_cache.borrow();
        for index in 0..40 {
            let entry = cache.get_entry(index).expect("cached layout");
            let nominal = entry.trailing_spacing_after.expect("space-after");
            let consumed = recorded_spacing_after(entry.trailing_spacing_after_consumed)
                .expect("consumed space-after");
            assert!(consumed <= nominal + 1e-9, "paragraph {index}");
            if consumed + 0.01 < nominal {
                clamped += 1;
            }
        }
    }
    assert!(clamped > 0, "a space-after is clamped at a page foot");

    // Replaying a paragraph restores its consumed amount at its end.
    renderer.current_page_number = 1;
    renderer.current_y = 500.0;
    renderer.note_replayed_spacing_after(Some(3.0));
    assert_eq!(renderer.current_spacing_after_consumed(), Some(3.0));
    renderer.current_y = 520.0;
    assert_eq!(renderer.current_spacing_after_consumed(), None);
}

/// An edit that grows a centered, bottom-aligned or justified page lays out
/// exactly as a fresh render of the edited document, in one render. The
/// measuring pass used to start from the previous layout's page offsets,
/// so the grown page overflowed the body bottom and broke one page early.
#[test]
fn growing_an_aligned_page_matches_a_fresh_render() {
    let body = |first: &str| {
        let mut body = paragraph(first, "");
        for index in 1..30 {
            body.push_str(&paragraph(&format!("Line {index}"), ""));
        }
        body.push_str(&format!(r#"<w:sectPr>{MARGINS}</w:sectPr>"#));
        load(&body)
    };
    let long = "word ".repeat(120);
    for align in ["center", "bottom", "both"] {
        let original = with_vertical_alignment(&body("First"), align);
        let edited = with_vertical_alignment(&body(long.trim_end()), align);
        let mut renderer = rendered(&original, &Options::default());
        assert_eq!(renderer.current_page_number, 1, "{align}: one page");
        renderer.set_incremental_dirty_from(0);
        renderer
            .render(&edited, &Options::default())
            .expect("edited render");
        let fresh = rendered(&edited, &Options::default());
        assert_eq!(fresh.current_page_number, 1, "{align}: still one page");
        assert_eq!(
            flow_signature(&renderer),
            flow_signature(&fresh),
            "{align}: pagination"
        );
        let tops = |renderer: &CanvasRenderer<RecordingPainter>| -> Vec<f64> {
            renderer
                .selection
                .paragraphs()
                .iter()
                .flat_map(|paragraph| paragraph.lines.iter().map(|line| line.absolute_top))
                .collect()
        };
        let (after, reference) = (tops(&renderer), tops(&fresh));
        assert_eq!(after.len(), reference.len(), "{align}: line count");
        for (index, (edited, fresh)) in after.iter().zip(&reference).enumerate() {
            assert!(
                (edited - fresh).abs() < 0.01,
                "{align}: line {index} at {edited}, fresh {fresh}"
            );
        }
    }
}
