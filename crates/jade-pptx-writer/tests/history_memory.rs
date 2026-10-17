use jade_pptx_writer::{EditSession, RunAddress, TextInputEdit, TypingKind};
use sha2::{Digest, Sha256};

const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");
fn media_deck() -> Vec<u8> {
    let mut data = vec![0; 512 * 256 * 4];
    let mut seed = 17u32;
    for pixel in data.chunks_exact_mut(4) {
        for byte in &mut pixel[..3] {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *byte = seed as u8;
        }
        pixel[3] = 255;
    }
    let mut image = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut image, 512, 256);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&data)
            .unwrap();
    }
    jade_pptx_writer::insert_picture(
        DECK,
        0,
        jade_pptx_reader::Transform {
            offset_emu: (952500, 952500),
            extent_emu: (4876800, 2438400),
            rotation_60000ths: 0,
            flip_h: false,
            flip_v: false,
        },
        &image,
    )
    .unwrap()
    .0
}
fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[test]
fn one_hundred_media_backed_edits_keep_all_undo_bytes_without_one_archive_per_step() {
    let bytes = media_deck();
    assert!(bytes.len() > 350_000);
    let mut session = EditSession::open(&bytes).unwrap();
    let mut hashes = Vec::new();
    let address = RunAddress {
        shape_id: 2,
        paragraph: 0,
        run: 0,
    };
    for edit in 0..100 {
        hashes.push(digest(session.bytes()));
        session
            .set_run_text(0, address, &format!("history edit {edit}"))
            .unwrap();
    }
    assert_eq!(session.history_depth(), (100, 0));
    assert!(
        session.retained_archive_bytes() < bytes.len() * 3 + 100 * 4096,
        "{} bytes retained for {}-byte deck",
        session.retained_archive_bytes(),
        bytes.len()
    );
    let edited = digest(session.bytes());
    for hash in hashes.iter().rev() {
        assert!(session.undo());
        assert_eq!(digest(session.bytes()), *hash);
    }
    assert_eq!(session.bytes(), bytes);
    assert!(!session.is_dirty());
    assert!(!session.undo());
    for _ in 0..100 {
        assert!(session.redo());
    }
    assert_eq!(digest(session.bytes()), edited);
    assert!(!session.redo());
    session.mark_saved();
    assert!(!session.is_dirty());
    session.undo();
    assert!(session.is_dirty());
    session.redo();
    assert!(!session.is_dirty());
}

#[test]
fn coalescing_and_branch_discard_release_obsolete_payloads_and_keep_saved_baseline() {
    let bytes = media_deck();
    let mut session = EditSession::open(&bytes).unwrap();
    let mut text = jade_pptx_writer::shape_text(session.bytes(), 0, 2)
        .unwrap()
        .text;
    for _ in 0..40 {
        let offset = text.encode_utf16().count();
        let result = session
            .apply_text_input(
                0,
                &TextInputEdit {
                    shape_id: 2,
                    start: offset,
                    end: offset,
                    text: "a".into(),
                    expected_text: text.clone(),
                    soft_break: false,
                    history: Some(TypingKind::Insert),
                    format: Default::default(),
                },
            )
            .unwrap();
        assert!(result.changed);
        text.push('a');
    }
    assert_eq!(session.history_depth(), (1, 0));
    assert!(session.retained_archive_bytes() < bytes.len() * 3 + 16384);
    assert!(session.undo());
    assert_eq!(session.bytes(), bytes);
    session.set_shape_text(0, 2, "new branch", false).unwrap();
    assert!(!session.can_redo());
    session.mark_saved();
    let saved = session.bytes().to_vec();
    for i in 0..105 {
        session
            .set_shape_text(0, 2, &format!("branch {i}"), false)
            .unwrap();
    }
    assert_eq!(session.history_depth(), (100, 0));
    assert!(session.is_dirty());
    assert!(session.retained_archive_bytes() < bytes.len() * 3 + 100 * 4096);
    for _ in 0..100 {
        assert!(session.undo());
    }
    assert_ne!(session.bytes(), saved);
    assert!(session.is_dirty());
}
