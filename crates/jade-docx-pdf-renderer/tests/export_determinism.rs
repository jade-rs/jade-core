//! Locks the exact PDF bytes produced for a few multi-page fixtures so
//! performance refactors of the export path can be shown byte-identical.
//!
//! Set `PDF_HASH_PRINT=1` to print the current values instead of asserting.

use jade_docx_pdf_renderer::{RenderOptions, TextMeasurement, render_docx_to_pdf};
use std::fs;
use std::path::PathBuf;

const CANDIDATES: &[&str] = &[
    "yokohama.docx",
    "contract.docx",
    "nda.docx",
    "trust-beneficiary-sale-keep-next.docx",
    "table-repeating-header.docx",
    "header-footer-sectionpages.docx",
    "image-inline.docx",
    "image-floating.docx",
    "table-complex-nested-pagination.docx",
];

/// (fixture, measurement is shaping, expected FNV-1a 64 of the PDF bytes)
/// Recorded with the pre-optimisation export code.
const EXPECTED: &[(&str, bool, u64)] = &[
    ("yokohama.docx", false, 0xf10f_7da1_6819_4b03),
    ("contract.docx", false, 0xc0c9_4741_ba2d_a2ac),
    (
        "trust-beneficiary-sale-keep-next.docx",
        false,
        0x971c_af8d_6cc3_5f77,
    ),
    (
        "trust-beneficiary-sale-keep-next.docx",
        true,
        0x50bd_a2a4_c5a0_e16a,
    ),
    ("table-repeating-header.docx", false, 0xdc8f_c32b_654d_6959),
    ("table-repeating-header.docx", true, 0x278b_4c65_99d1_9d81),
    (
        "header-footer-sectionpages.docx",
        false,
        0x54e1_cd0a_507e_6622,
    ),
    (
        "header-footer-sectionpages.docx",
        true,
        0x30d7_fd37_cfa3_1654,
    ),
    (
        "table-complex-nested-pagination.docx",
        false,
        0xb345_7803_84f3_2ef6,
    ),
    (
        "table-complex-nested-pagination.docx",
        true,
        0xa9cd_5a8f_69ca_8064,
    ),
];

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn export(name: &str, shaping: bool) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(name);
    let bytes = fs::read(&path).expect("fixture should exist");
    let options = RenderOptions {
        text_measurement: if shaping {
            TextMeasurement::BrowserParityShaping
        } else {
            TextMeasurement::EmbeddedFontAdvances
        },
        ..RenderOptions::default()
    };
    render_docx_to_pdf(&bytes, &options).expect("fixture should render")
}

#[test]
fn print_candidate_hashes() {
    if std::env::var_os("PDF_HASH_PRINT").is_none() {
        return;
    }
    for name in CANDIDATES {
        for shaping in [false, true] {
            let first = export(name, shaping);
            let second = export(name, shaping);
            let pages = lopdf::Document::load_mem(&first)
                .map(|doc| doc.get_pages().len())
                .unwrap_or(0);
            println!(
                "HASH {name} shaping={shaping} pages={pages} len={} fnv={:#018x} deterministic={}",
                first.len(),
                fnv1a(&first),
                first == second
            );
        }
    }
}

#[test]
fn multi_page_fixture_exports_are_byte_stable() {
    for (name, shaping, expected) in EXPECTED {
        let first = export(name, *shaping);
        let second = export(name, *shaping);
        assert_eq!(first, second, "{name} export must be deterministic");
        assert_eq!(
            fnv1a(&first),
            *expected,
            "{name} (shaping={shaping}) PDF bytes changed"
        );
    }
}
