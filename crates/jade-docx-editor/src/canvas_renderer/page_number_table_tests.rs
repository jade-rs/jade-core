//! The per-section page-number table (`CanvasRenderer::page_number_table`)
//! against the algorithm it replaced, which walked the sections and scanned
//! the pages for each one on every `PAGE` lookup.

use super::tests::test_renderer;
use super::*;
use crate::document::PageNumberType;

/// xorshift64*: deterministic, so a failure names its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound.max(1) as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

fn numbering(rng: &mut Rng, sections: usize) -> Vec<Option<PageNumberType>> {
    (0..sections)
        .map(|_| match rng.below(4) {
            0 => None,
            1 => Some(PageNumberType::default()),
            _ => Some(PageNumberType {
                start: Some(rng.below(20) as u32),
                ..PageNumberType::default()
            }),
        })
        .collect()
}

/// Every page (and the one after the last) through the table and through
/// the reference, with the renderer's current layout state.
fn assert_matches_reference(renderer: &CanvasRenderer, seed: u64) {
    for page in 0..=renderer.current_page_number + 1 {
        assert_eq!(
            renderer.page_number_value_from_table(page),
            renderer.page_number_value_reference(page),
            "seed {seed}, page {page}, state {:?} / {:?}",
            renderer.page_section_indices,
            renderer.mid_page_section_starts,
        );
    }
}

/// Header, footer and note stories read the shared page's numbering with a
/// started-but-uncommitted paragraph counting differently, so check those
/// states too.
fn assert_matches_reference_in_every_story(renderer: &mut CanvasRenderer, seed: u64) {
    assert_matches_reference(renderer, seed);
    renderer.rendering_header_footer_story = true;
    assert_matches_reference(renderer, seed);
    renderer.rendering_header_footer_story = false;
    renderer.rendering_note_story = true;
    assert_matches_reference(renderer, seed);
    renderer.rendering_note_story = false;
}

/// Lays a document out step by step through the renderer's own bookkeeping
/// (next-page and continuous breaks, mid-page content, starts), checking
/// every page after each step so a stale table would show.
#[test]
fn the_table_matches_the_reference_across_random_layouts() {
    for seed in 1..=400u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut renderer = test_renderer();
        let sections = 1 + rng.below(8);
        renderer.set_section_page_numbering(numbering(&mut rng, sections));
        renderer.current_page_number = 1;
        renderer.active_section_index = 0;
        renderer.record_page_section(1);
        for _ in 0..rng.below(60) {
            let page = renderer.current_page_number;
            let can_break = renderer.active_section_index + 1 < sections;
            match rng.below(10) {
                0..=3 => {
                    renderer.current_page_number = page + 1;
                    renderer.record_page_section(page + 1);
                }
                4 if can_break => {
                    // Next-page break.
                    renderer.settle_mid_page_section_content(renderer.active_section_index);
                    renderer.active_section_index += 1;
                    renderer.current_page_number = page + 1;
                    renderer.record_page_section(page + 1);
                }
                5 | 6 if can_break => {
                    // Continuous break, as `switch_to_section` records it.
                    let page_top_section = renderer.section_index_for_page(page);
                    renderer.settle_mid_page_section_content(renderer.active_section_index);
                    let target = renderer.active_section_index + 1;
                    renderer.set_mid_page_section_start(target, None);
                    renderer.active_section_index = target;
                    renderer.record_mid_page_section_start(page_top_section);
                }
                7 => renderer.note_mid_page_section_content(renderer.active_section_index),
                8 => renderer.note_mid_page_section_line(renderer.active_section_index),
                _ => {
                    // A seek or a later pass restoring an earlier pagination.
                    let keep = 1 + rng.below(renderer.page_section_indices.len());
                    renderer.page_section_indices.truncate(keep);
                    renderer.invalidate_page_number_table();
                }
            }
            assert_matches_reference_in_every_story(&mut renderer, seed);
        }
    }
}

/// Arbitrary, even inconsistent, ownership and mid-page data: the table
/// must still agree with the reference everywhere.
#[test]
fn the_table_matches_the_reference_for_arbitrary_section_data() {
    let contents = [
        MidPageContent::Pending,
        MidPageContent::ParagraphStarted,
        MidPageContent::Placed,
        MidPageContent::Absent,
    ];
    for seed in 1..=2000u64 {
        let mut rng = Rng(seed.wrapping_mul(0xd1b5_4a32_d192_ed03) | 1);
        let mut renderer = test_renderer();
        let sections = 1 + rng.below(10);
        let pages = rng.below(25);
        renderer.set_section_page_numbering(numbering(&mut rng, sections));
        let mut owner = 0;
        renderer.page_section_indices = (0..pages)
            .map(|_| {
                if rng.chance(30) {
                    owner = rng.below(sections + 2);
                }
                owner
            })
            .collect();
        renderer.mid_page_section_starts = (0..rng.below(sections + 2))
            .map(|_| {
                rng.chance(40).then(|| MidPageSectionStart {
                    page: 1 + rng.below(pages + 1),
                    page_top_section: rng.below(sections),
                    content: contents[rng.below(contents.len())],
                })
            })
            .collect();
        renderer.current_page_number = rng.below(pages + 2);
        renderer.active_section_index = rng.below(sections + 2);
        renderer.invalidate_page_number_table();
        assert_matches_reference_in_every_story(&mut renderer, seed);
    }
}

/// 200 pages in 20 sections, every other one restarting: resolving every
/// page's number visits the page list once (building the table) plus one
/// element for each page of section 0, where the reference scans it per
/// section per page.
#[test]
fn page_labels_do_not_scan_the_pages_per_lookup() {
    const PAGES: usize = 200;
    const SECTIONS: usize = 20;
    let mut renderer = test_renderer();
    renderer.set_section_page_numbering(
        (0..SECTIONS)
            .map(|section| {
                Some(PageNumberType {
                    start: (section % 2 == 1).then_some(1),
                    ..PageNumberType::default()
                })
            })
            .collect(),
    );
    renderer.page_section_indices = (0..PAGES).map(|page| page / (PAGES / SECTIONS)).collect();
    renderer.current_page_number = PAGES;
    renderer.active_section_index = SECTIONS - 1;
    renderer.page_number_reference_check = false;

    renderer.page_number_page_visits.set(0);
    // `page_labels` over the 200 pages (the test host reports one page).
    let labels: Vec<(usize, usize)> = (1..=PAGES)
        .map(|page| {
            (
                renderer.numbering_section_for_page(page),
                renderer.page_number_value(page),
            )
        })
        .collect();
    assert_eq!(renderer.page_labels(), labels[..1]);
    let displayed: Vec<String> = (1..=PAGES)
        .map(|page| renderer.displayed_page_number(page))
        .collect();
    let visits = renderer.page_number_page_visits.get();
    // One table build, plus one element per section-0 lookup: the labels
    // and the displayed numbers each read section 0's 10 pages, and
    // `page_labels` reads page 1. The old walk visited about 20,000.
    let section_zero_pages = PAGES / SECTIONS;
    assert!(
        visits <= PAGES + 2 * section_zero_pages + 1,
        "{visits} page visits for {PAGES} pages"
    );

    assert_eq!(labels.len(), PAGES);
    assert_eq!(labels[0], (0, 1));
    assert_eq!(labels[10], (1, 1), "section 1 restarts at 1");
    assert_eq!(labels[20], (2, 11), "section 2 continues");
    assert_eq!(displayed[21], "12");
    for page in 1..=PAGES {
        assert_eq!(
            renderer.page_number_value_from_table(page),
            renderer.page_number_value_reference(page),
            "page {page}"
        );
    }
}

/// `cargo test -p jade-docx-editor --lib page_number_table_timing -- --ignored --nocapture`
#[test]
#[ignore = "timing only"]
fn page_number_table_timing() {
    for (pages, sections) in [(200usize, 20usize), (2000, 200)] {
        let mut renderer = test_renderer();
        renderer.set_section_page_numbering(
            (0..sections)
                .map(|section| {
                    Some(PageNumberType {
                        start: (section % 3 == 0).then_some(1),
                        ..PageNumberType::default()
                    })
                })
                .collect(),
        );
        renderer.page_section_indices = (0..pages).map(|page| page * sections / pages).collect();
        renderer.current_page_number = pages;
        renderer.active_section_index = sections - 1;
        renderer.page_number_reference_check = false;

        let started = std::time::Instant::now();
        let table: usize = (1..=pages)
            .map(|page| renderer.page_number_value_from_table(page))
            .sum();
        let table_time = started.elapsed();
        let started = std::time::Instant::now();
        let reference: usize = (1..=pages)
            .map(|page| renderer.page_number_value_reference(page))
            .sum();
        let reference_time = started.elapsed();
        assert_eq!(table, reference);
        eprintln!(
            "{pages} pages x {sections} sections: table {table_time:?}, reference {reference_time:?}"
        );
    }
}
