//! 自動回復 and バックアップ as Word's 環境設定 → 保存 sets them, as far as
//! a shell without a file system shares them: the settings and the text
//! they are kept in, when a recovery copy is due, what a backup is called,
//! and the copies a later visit offers back, newest first.

/// The save settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoverySettings {
    /// 自動回復用データを保存する.
    pub enabled: bool,
    /// How often, in minutes (1–120).
    pub minutes: u32,
    /// バックアップ ファイルを常に作成する.
    pub always_backup: bool,
}

impl Default for RecoverySettings {
    /// Word's: every 10 minutes, no backups.
    fn default() -> Self {
        RecoverySettings {
            enabled: true,
            minutes: 10,
            always_backup: false,
        }
    }
}

/// The intervals 自動回復 offers, in minutes.
pub const INTERVALS: [u32; 6] = [1, 5, 10, 15, 30, 60];

impl RecoverySettings {
    /// The settings in `text` (`key=value` lines, the native app's
    /// `save.txt`), Word's defaults for what it does not say.
    pub fn parse(text: &str) -> Self {
        let mut settings = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "autorecover" => settings.enabled = value == "on",
                "minutes" => {
                    if let Ok(minutes) = value.parse::<u32>() {
                        settings.minutes = minutes.clamp(1, 120);
                    }
                }
                "backup" => settings.always_backup = value == "on",
                _ => {}
            }
        }
        settings
    }

    pub fn format(&self) -> String {
        let on = |value: bool| if value { "on" } else { "off" };
        format!(
            "autorecover={}\nminutes={}\nbackup={}\n",
            on(self.enabled),
            self.minutes,
            on(self.always_backup),
        )
    }

    /// Whether a copy is due at `now`, the last made at `last` (both in
    /// milliseconds), for a document with unsaved edits or none.
    pub fn due(&self, unsaved: bool, last: Option<f64>, now: f64) -> bool {
        self.enabled
            && unsaved
            && last.is_none_or(|last| now - last >= f64::from(self.minutes) * 60_000.0)
    }
}

/// What the backup of `name` is called: `<stem> のバックアップ.wbk`, or
/// `Backup of <stem>.wbk` in English, as Word names it.
pub fn backup_name(name: &str, japanese: bool) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    if japanese {
        format!("{stem} のバックアップ.wbk")
    } else {
        format!("Backup of {stem}.wbk")
    }
}

/// A recovery copy, as a later visit lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct RecoveredCopy {
    /// Its key in the store.
    pub key: String,
    /// The document it is a copy of.
    pub name: String,
    /// When it was made, in milliseconds since the epoch.
    pub saved_at: f64,
}

/// The copies to offer back, newest first, the copies of the documents
/// open now left out (they are this visit's own).
pub fn to_offer(mut copies: Vec<RecoveredCopy>, open: &[String]) -> Vec<RecoveredCopy> {
    copies.retain(|copy| !open.contains(&copy.key));
    copies.sort_by(|a, b| b.saved_at.total_cmp(&a.saved_at));
    copies
}

/// The name a recovered copy opens under: Word's `<stem> (回復済み).docx`.
pub fn recovered_name(name: &str, japanese: bool) -> String {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    if japanese {
        format!("{stem} (回復済み).docx")
    } else {
        format!("{stem} (Recovered).docx")
    }
}
