use jade_opc::{ArchiveSnapshot, SnapshotStore};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};

fn archive(payload: &[u8], edit: usize, deflated: bool) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(if deflated {
        zip::CompressionMethod::Deflated
    } else {
        zip::CompressionMethod::Stored
    });
    writer.start_file("slide.xml", options).unwrap();
    write!(writer, "<text>{edit}</text>").unwrap();
    writer.start_file("media.bin", options).unwrap();
    writer.write_all(payload).unwrap();
    writer.set_comment(format!("comment {edit}"));
    writer.finish().unwrap().into_inner()
}

#[test]
fn snapshots_share_shifted_payloads_and_restore_every_original_zip_byte() {
    let payload = (0..1024 * 1024)
        .map(|n| (n % 251) as u8)
        .collect::<Vec<_>>();
    let mut pool = SnapshotStore::default();
    let mut snapshots: Vec<(ArchiveSnapshot, [u8; 32])> = Vec::new();
    for edit in 0..100 {
        let bytes = archive(&payload, edit, false);
        let digest = Sha256::digest(&bytes).into();
        snapshots.push((pool.capture(&bytes), digest));
    }
    assert!(
        pool.retained_bytes() < payload.len() + 100 * 1024,
        "{} bytes",
        pool.retained_bytes()
    );
    for (snapshot, digest) in &snapshots {
        assert_eq!(
            <[u8; 32]>::from(Sha256::digest(snapshot.restore())),
            *digest
        );
    }
    let last = snapshots.pop().unwrap();
    drop(snapshots);
    assert!(pool.retained_bytes() < payload.len() + 1024);
    drop(last);
    assert_eq!(pool.retained_bytes(), 0);
}

#[test]
fn comments_prefixes_trailing_data_compression_and_opaque_fallback_round_trip() {
    let mut pool = SnapshotStore::default();
    for deflated in [false, true] {
        let bytes = archive(b"unchanged media", 12, deflated);
        for bytes in [
            bytes.clone(),
            [b"MZ-prefix".as_slice(), bytes.as_slice(), b"trailing-data"].concat(),
        ] {
            let snapshot = pool.capture(&bytes);
            assert_eq!(snapshot.restore(), bytes);
            assert!(snapshot == pool.capture(&bytes));
            let changed = archive(b"different media", 12, deflated);
            assert!(snapshot != pool.capture(&changed));
        }
    }
    for bytes in [b"not a zip".as_slice(), b"", b"PK\x03\x04incomplete"] {
        let snapshot = pool.capture(bytes);
        assert_eq!(snapshot.restore(), bytes);
        assert!(snapshot == pool.capture(bytes));
    }
}
