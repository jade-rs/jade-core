//! `OpcPackage::open` is the one door XLSX and PPTX come in through, so it is
//! where a hostile archive has to be stopped.

use jade_opc::{OpcError, OpcPackage, PackageLoadLimits};
use std::io::{Cursor, Write};

fn zip(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, bytes) in parts {
        zip.start_file(*name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

fn limit_resource(bytes: &[u8], limits: PackageLoadLimits) -> &'static str {
    match OpcPackage::open_with_limits(bytes, limits) {
        Err(OpcError::Limit(limit)) => {
            assert!(limit.actual > limit.limit);
            limit.resource
        }
        Err(other) => panic!("expected a limit error, got {other}"),
        Ok(_) => panic!("expected the package to be refused"),
    }
}

#[test]
fn archive_budget_is_checked_before_zip_parsing() {
    let limits = PackageLoadLimits {
        max_archive_bytes: 4,
        ..Default::default()
    };
    assert_eq!(limit_resource(&[0; 5], limits), "archive bytes");
}

#[test]
fn entry_budget_rejects_many_small_parts() {
    let bytes = zip(&[("a.xml", b"a"), ("b.xml", b"b")]);
    let limits = PackageLoadLimits {
        max_entries: 1,
        ..Default::default()
    };
    assert_eq!(limit_resource(&bytes, limits), "entries");
}

/// The shape of a ZIP bomb: a part that compresses to almost nothing and
/// inflates past the budget.
#[test]
fn part_budget_rejects_compressed_expansion() {
    let inflated = vec![b'x'; 1024 * 1024];
    let bytes = zip(&[("xl/worksheets/sheet1.xml", &inflated)]);
    assert!(
        bytes.len() < 8 * 1024,
        "the fixture must actually be a bomb"
    );
    let limits = PackageLoadLimits {
        max_part_bytes: 512 * 1024,
        ..Default::default()
    };
    assert_eq!(limit_resource(&bytes, limits), "part bytes");
}

#[test]
fn total_budget_applies_across_parts() {
    let bytes = zip(&[("a.xml", b"123"), ("b.xml", b"456")]);
    let limits = PackageLoadLimits {
        max_part_bytes: 3,
        max_total_bytes: 5,
        ..Default::default()
    };
    assert_eq!(limit_resource(&bytes, limits), "expanded bytes");
}

#[test]
fn exact_boundaries_keep_every_part() {
    let bytes = zip(&[("a.xml", b"123"), ("b.xml", b"456")]);
    let package = OpcPackage::open_with_limits(
        &bytes,
        PackageLoadLimits {
            max_archive_bytes: bytes.len() as u64,
            max_entries: 2,
            max_part_bytes: 3,
            max_total_bytes: 6,
        },
    )
    .unwrap();
    assert_eq!(package.part("a.xml"), Some(&b"123"[..]));
    assert_eq!(package.part("b.xml"), Some(&b"456"[..]));
}

/// The defaults must not refuse anything real: every XLSX / PPTX fixture in the
/// repository opens through `open` unchanged.
#[test]
fn repository_fixtures_fit_default_budgets() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../public/fixtures");
    let mut checked = 0;
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy();
        if name.starts_with("._") {
            continue;
        }
        if path
            .extension()
            .is_some_and(|ext| ext == "xlsx" || ext == "pptx")
        {
            let bytes = std::fs::read(&path).unwrap();
            OpcPackage::open(&bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            checked += 1;
        }
    }
    assert!(checked > 0);
}

#[test]
fn filenames_decode_utf8_and_cp437() {
    let utf8 = zip(&[("資料.xml", b"contents")]);
    assert_eq!(
        OpcPackage::open(&utf8).unwrap().part("資料.xml").unwrap(),
        b"contents"
    );
    let mut legacy = zip(&[("a.xml", b"contents")]);
    let offsets: Vec<_> = legacy
        .windows(5)
        .enumerate()
        .filter_map(|(offset, name)| (name == b"a.xml").then_some(offset))
        .collect();
    assert_eq!(offsets.len(), 2);
    for offset in offsets {
        legacy[offset] = 0x82;
    }
    assert_eq!(
        OpcPackage::open(&legacy).unwrap().part("é.xml").unwrap(),
        b"contents"
    );
}

#[test]
fn corrupt_archive_returns_error() {
    assert!(OpcPackage::open(b"not a ZIP archive").is_err());
}
