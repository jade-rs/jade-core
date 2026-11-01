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

/// Independent cold decoder: this path never reads the OPC inflated cache.
fn cold_decode(bytes: &[u8]) -> zip::result::ZipResult<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
    let mut entry = archive.by_index(0)?;
    let mut contents = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut contents)?;
    Ok(contents)
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

#[test]
fn identical_raw_bytes_with_a_different_compression_method_are_not_the_same_part() {
    let name = "xl/cache-method-identity.xml";
    let original = noisy(531, 100_000);
    let deflated = package(&[(name, original.clone())]);
    let compressed = {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&deflated)).unwrap();
        let mut entry = archive.by_index_raw(0).unwrap();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut bytes).unwrap();
        bytes
    };
    assert!(compressed.len() >= 16 * 1024);
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file(
            name,
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    std::io::Write::write_all(&mut writer, &compressed).unwrap();
    let stored = writer.finish().unwrap().into_inner();
    // Both archives are valid and have exactly the same raw entry bytes.
    // Their decoded contents differ because their methods differ.
    let first = OpcPackage::open(&deflated).unwrap();
    assert_eq!(first.part(name), Some(original.as_slice()));
    let second = OpcPackage::open(&stored).unwrap();
    assert_eq!(second.part(name).map(<[u8]>::len), Some(compressed.len()));
    assert_eq!(second.part(name), Some(compressed.as_slice()));
}

#[test]
fn a_cached_part_must_not_hide_a_corrupt_crc() {
    let original = package(&[("xl/cache-crc-identity.xml", noisy(532, 100_000))]);
    OpcPackage::open(&original).unwrap();
    let mut corrupt = original.clone();
    let central = corrupt
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    corrupt[14] ^= 1; // local CRC
    corrupt[central + 16] ^= 1; // central-directory CRC
    assert!(cold_decode(&corrupt).is_err());
    assert!(
        OpcPackage::open(&corrupt).is_err(),
        "a warm cache bypassed CRC validation"
    );
}

#[test]
fn a_cached_part_must_not_hide_encryption_flags() {
    let original = package(&[("xl/cache-encryption-identity.xml", noisy(533, 100_000))]);
    OpcPackage::open(&original).unwrap();
    let mut encrypted = original;
    let central = encrypted
        .windows(4)
        .position(|bytes| bytes == b"PK\x01\x02")
        .unwrap();
    encrypted[6] |= 1; // local encrypted flag
    encrypted[central + 8] |= 1; // central encrypted flag
    assert!(cold_decode(&encrypted).is_err());
    assert!(
        OpcPackage::open(&encrypted).is_err(),
        "a warm cache bypassed encryption handling"
    );
}
