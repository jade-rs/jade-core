//! Opening what an edit just wrote gives its parts back exactly, large ones
//! from what the write kept, within the same budgets as inflating.

use jade_opc::package_limits::PackageLoadLimits;
use jade_opc::OpcPackage;

/// Text that deflates poorly, so its part is large compressed too.
fn noisy(seed: u32, len: usize) -> Vec<u8> {
    let mut state = seed;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            b'a' + (state >> 16) as u8 % 26
        })
        .collect()
}

fn package(parts: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
        for (name, contents) in parts {
            writer
                .start_file(
                    *name,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .unwrap();
            std::io::Write::write_all(&mut writer, contents).unwrap();
        }
        writer.finish().unwrap();
    }
    buffer
}

#[test]
fn a_written_large_part_opens_as_written_and_still_counts_against_the_budget() {
    let original = package(&[
        ("[Content_Types].xml", b"<Types/>".to_vec()),
        ("xl/worksheets/sheet1.xml", noisy(1, 200_000)),
        ("xl/small.xml", b"<small/>".to_vec()),
    ]);
    for round in 2..5 {
        let source = original.clone();
        let mut editing = OpcPackage::open_preserving(&source).unwrap();
        let edited = noisy(round, 200_000);
        editing.replace("xl/worksheets/sheet1.xml", edited.clone());
        let written = editing.write().unwrap();
        // Twice: the second open finds what the first kept.
        for _ in 0..2 {
            let reopened = OpcPackage::open(&written).unwrap();
            assert_eq!(reopened.part("xl/worksheets/sheet1.xml"), Some(&edited[..]));
            assert_eq!(reopened.part("xl/small.xml"), Some(&b"<small/>"[..]));
        }
        // A kept part is no way round the budgets.
        let tight = PackageLoadLimits {
            max_total_bytes: 100_000,
            ..PackageLoadLimits::default()
        };
        assert!(OpcPackage::open_with_limits(&written, tight).is_err());
    }
}

#[test]
fn a_different_archive_with_the_same_part_name_is_inflated_for_itself() {
    let first = package(&[("xl/worksheets/sheet1.xml", noisy(7, 100_000))]);
    let second = package(&[("xl/worksheets/sheet1.xml", noisy(8, 100_000))]);
    let a = OpcPackage::open(&first).unwrap();
    let b = OpcPackage::open(&second).unwrap();
    assert_eq!(
        a.part("xl/worksheets/sheet1.xml"),
        Some(&noisy(7, 100_000)[..])
    );
    assert_eq!(
        b.part("xl/worksheets/sheet1.xml"),
        Some(&noisy(8, 100_000)[..])
    );
}
