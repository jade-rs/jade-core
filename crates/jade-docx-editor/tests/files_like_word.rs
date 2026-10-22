//! Word's file tools as both shells share them: 自動回復's settings and
//! when a copy is due, what a backup and a recovered copy are called, the
//! copies a later visit offers, and 互換性チェック's report on a Word 2010
//! document before and after 変換.

use jade_docx_editor::word::compat::{self, WordVersion};
use jade_docx_editor::word::recovery::{
    backup_name, recovered_name, to_offer, RecoveredCopy, RecoverySettings,
};

#[test]
fn autorecover_settings_round_trip_and_say_when_a_copy_is_due() {
    let settings = RecoverySettings::default();
    assert!(settings.enabled);
    assert_eq!(settings.minutes, 10);
    let changed = RecoverySettings {
        enabled: true,
        minutes: 5,
        always_backup: true,
    };
    assert_eq!(RecoverySettings::parse(&changed.format()), changed);
    // Out of range minutes are held to Word's.
    assert_eq!(RecoverySettings::parse("minutes=999").minutes, 120);
    // Due only with unsaved edits, once the interval has gone by.
    assert!(changed.due(true, None, 0.0));
    assert!(!changed.due(false, None, 0.0));
    assert!(!changed.due(true, Some(0.0), 4.0 * 60_000.0));
    assert!(changed.due(true, Some(0.0), 5.0 * 60_000.0));
    let off = RecoverySettings {
        enabled: false,
        ..changed
    };
    assert!(!off.due(true, None, 0.0));
}

#[test]
fn backups_and_recovered_copies_are_named_as_word_names_them() {
    assert_eq!(
        backup_name("契約書.docx", true),
        "契約書 のバックアップ.wbk"
    );
    assert_eq!(backup_name("report.docx", false), "Backup of report.wbk");
    assert_eq!(recovered_name("a.docx", true), "a (回復済み).docx");
    assert_eq!(recovered_name("a.docx", false), "a (Recovered).docx");
    let copy = |key: &str, saved_at: f64| RecoveredCopy {
        key: key.to_string(),
        name: format!("{key}.docx"),
        saved_at,
    };
    let offered = to_offer(
        vec![copy("old", 1.0), copy("open", 3.0), copy("new", 2.0)],
        &["open".to_string()],
    );
    let keys: Vec<_> = offered.iter().map(|copy| copy.key.as_str()).collect();
    assert_eq!(keys, ["new", "old"]);
}

#[test]
fn a_word_2010_document_is_checked_and_converted() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../public/fixtures/word-compat-comprehensive.docx"
    ))
    .unwrap();
    let mode = compat::compatibility_mode(&bytes);
    assert_eq!(mode, 14);
    assert!(compat::is_compatibility_mode(mode));
    assert_eq!(WordVersion::of_mode(mode), WordVersion::Word2010);
    // What Word 97-2003 would lose is at least what Word 2010 would.
    let older = compat::check(&bytes, WordVersion::Word2003);
    let newer = compat::check(&bytes, WordVersion::Word2010);
    assert!(older.len() >= newer.len());
    let upgraded = compat::upgrade(&bytes).unwrap();
    assert_eq!(compat::compatibility_mode(&upgraded), compat::CURRENT_MODE);
}
