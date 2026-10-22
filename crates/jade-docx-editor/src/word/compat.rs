//! 互換モード and the 互換性チェック, as Word has them.
//!
//! A document saved by an older Word carries the version it was laid out
//! for (`w:compatSetting w:name="compatibilityMode"`: 11 for Word 2003, 12
//! for 2007, 14 for 2010, 15 for 2013 and later). Word opens such a
//! document in 互換モード — the title says so — and lays it out as that
//! version did; 変換 upgrades it to the current mode, after which it is laid
//! out as a current document is. A Word 97–2004 file is in Word 2003's mode.
//!
//! The checker lists what the document has that an older version does not
//! keep: what would be lost or changed if it were opened there.

use jade_opc::OpcPackage;

/// The current compatibility mode: Word 2013 and later.
pub const CURRENT_MODE: u32 = 15;

/// The versions the checker can check for, with the mode each lays out in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WordVersion {
    /// Word 97–2003 (`.doc`).
    Word2003,
    Word2007,
    Word2010,
}

impl WordVersion {
    pub const ALL: [WordVersion; 3] = [
        WordVersion::Word2003,
        WordVersion::Word2007,
        WordVersion::Word2010,
    ];

    pub fn mode(self) -> u32 {
        match self {
            WordVersion::Word2003 => 11,
            WordVersion::Word2007 => 12,
            WordVersion::Word2010 => 14,
        }
    }

    /// The version a mode is, the newest one at or before it.
    pub fn of_mode(mode: u32) -> Self {
        match mode {
            0..=11 => WordVersion::Word2003,
            12..=13 => WordVersion::Word2007,
            _ => WordVersion::Word2010,
        }
    }

    pub fn label(self, japanese: bool) -> &'static str {
        match (self, japanese) {
            (WordVersion::Word2003, true) => "Word 97-2003",
            (WordVersion::Word2003, false) => "Word 97-2003",
            (WordVersion::Word2007, _) => "Word 2007",
            (WordVersion::Word2010, _) => "Word 2010",
        }
    }
}

/// The compatibility mode `docx` is laid out in: its setting, or Word
/// 2007's when it has none (what a document from before Word 2010 says by
/// saying nothing).
pub fn compatibility_mode(docx: &[u8]) -> u32 {
    let Some(settings) = OpcPackage::open(docx)
        .ok()
        .and_then(|package| package.part_str("word/settings.xml").ok().flatten())
    else {
        return 12;
    };
    mode_in(&settings).unwrap_or(12)
}

fn mode_in(settings: &str) -> Option<u32> {
    let at = settings.find(r#"w:name="compatibilityMode""#)?;
    // The element around the name: from its `<` to its `>`.
    let start = settings[..at].rfind('<')?;
    let end = at + settings[at..].find('>')?;
    let element = &settings[start..end];
    let value = element.split(r#"w:val=""#).nth(1)?.split('"').next()?;
    value.trim().parse().ok()
}

/// Whether a document in `mode` opens in 互換モード.
pub fn is_compatibility_mode(mode: u32) -> bool {
    mode < CURRENT_MODE
}

/// The compatibility settings Word 2013 writes, with the mode first.
const CURRENT_SETTINGS: &str = r#"<w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/><w:compatSetting w:name="overrideTableStyleFontSizeAndJustification" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/><w:compatSetting w:name="enableOpenTypeFeatures" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/><w:compatSetting w:name="doNotFlipMirrorIndents" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/><w:compatSetting w:name="differentiateMultirowTableHeaders" w:uri="http://schemas.microsoft.com/office/word" w:val="1"/>"#;

/// 変換: `docx` upgraded to the current mode, as Word's 変換 makes it —
/// the mode set to 15 and Word 2013's settings in place of the old
/// version's options, so it is laid out as a current document.
pub fn upgrade(docx: &[u8]) -> Result<Vec<u8>, String> {
    let mut package = OpcPackage::open(docx).map_err(|error| error.to_string())?;
    let settings = package
        .part_str("word/settings.xml")
        .map_err(|error| error.to_string())?
        .unwrap_or_else(|| {
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"></w:settings>"#
                .to_string()
        });
    let compat = format!("<w:compat>{CURRENT_SETTINGS}</w:compat>");
    let upgraded = match (settings.find("<w:compat>"), settings.find("<w:compat/>")) {
        (Some(start), _) => {
            let end = settings[start..]
                .find("</w:compat>")
                .map(|end| start + end + "</w:compat>".len())
                .ok_or("the settings are damaged")?;
            format!("{}{compat}{}", &settings[..start], &settings[end..])
        }
        (None, Some(start)) => format!(
            "{}{compat}{}",
            &settings[..start],
            &settings[start + "<w:compat/>".len()..]
        ),
        (None, None) => {
            let end = settings
                .rfind("</w:settings>")
                .ok_or("the settings are damaged")?;
            format!("{}{compat}{}", &settings[..end], &settings[end..])
        }
    };
    package.replace("word/settings.xml", upgraded.into_bytes());
    // A settings part the package did not list needs its type and its
    // relationship.
    if !package
        .part_str("word/_rels/document.xml.rels")
        .ok()
        .flatten()
        .is_some_and(|rels| rels.contains("/settings\""))
    {
        if let Some(rels) = package
            .part_str("word/_rels/document.xml.rels")
            .ok()
            .flatten()
        {
            let added = rels.replace(
                "</Relationships>",
                r#"<Relationship Id="rIdJadeSettings" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/></Relationships>"#,
            );
            package.replace("word/_rels/document.xml.rels", added.into_bytes());
        }
        if let Some(types) = package.part_str("[Content_Types].xml").ok().flatten() {
            if !types.contains("/word/settings.xml") {
                let added = types.replace(
                    "</Types>",
                    r#"<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/></Types>"#,
                );
                package.replace("[Content_Types].xml", added.into_bytes());
            }
        }
    }
    package.write().map_err(|error| error.to_string())
}

/// Something an older Word does not keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Issue {
    /// Replies to comments become comments of their own.
    CommentReplies,
    /// Repeating-section content controls become plain ones.
    RepeatingSections,
    /// Content controls lose their colour and appearance.
    ContentControlAppearance,
    /// Online videos become pictures.
    OnlineVideos,
    /// Headings lose being collapsed.
    CollapsedHeadings,
    /// Text effects (glow, shadow, reflection, outline, gradient fill) are
    /// removed.
    TextEffects,
    /// OpenType features (ligatures, number forms, stylistic sets) are
    /// removed.
    OpenTypeFeatures,
    /// Check box content controls become static.
    CheckBoxes,
    /// Shapes and text boxes are converted to the older kind.
    Shapes,
    /// Picture effects and artistic effects are removed.
    PictureEffects,
    /// Tables lose their alternative text.
    TableAltText,
    /// Sizes and positions relative to the page become absolute.
    RelativeSizes,
    /// Equations become pictures that cannot be edited.
    Equations,
    /// Content controls become static text.
    ContentControls,
    /// SmartArt becomes a single picture.
    SmartArt,
    /// Charts become pictures.
    Charts,
    /// Tracked moves become insertions and deletions.
    TrackedMoves,
    /// Theme fonts and colours become fixed ones.
    Themes,
}

impl Issue {
    /// The newest version the issue applies to: it applies to that one and
    /// every older one.
    fn newest_affected(self) -> WordVersion {
        match self {
            Issue::CommentReplies
            | Issue::RepeatingSections
            | Issue::ContentControlAppearance
            | Issue::OnlineVideos
            | Issue::CollapsedHeadings => WordVersion::Word2010,
            Issue::TextEffects
            | Issue::OpenTypeFeatures
            | Issue::CheckBoxes
            | Issue::Shapes
            | Issue::PictureEffects
            | Issue::TableAltText
            | Issue::RelativeSizes => WordVersion::Word2007,
            Issue::Equations
            | Issue::ContentControls
            | Issue::SmartArt
            | Issue::Charts
            | Issue::TrackedMoves
            | Issue::Themes => WordVersion::Word2003,
        }
    }

    /// The markup that is the feature, and so counts it.
    fn markers(self) -> &'static [&'static str] {
        match self {
            Issue::CommentReplies => &["w15:paraIdParent="],
            Issue::RepeatingSections => &["<w15:repeatingSection"],
            Issue::ContentControlAppearance => &["<w15:appearance", "<w15:color"],
            Issue::OnlineVideos => &["<wp15:webVideoPr"],
            Issue::CollapsedHeadings => &["<w15:collapsed"],
            Issue::TextEffects => &[
                "<w14:glow",
                "<w14:shadow",
                "<w14:reflection",
                "<w14:textOutline",
                "<w14:textFill",
                "<w14:props3d",
                "<w14:scene3d",
            ],
            Issue::OpenTypeFeatures => &[
                "<w14:ligatures",
                "<w14:numForm",
                "<w14:numSpacing",
                "<w14:stylisticSets",
                "<w14:cntxtAlts",
            ],
            Issue::CheckBoxes => &["<w14:checkbox"],
            Issue::Shapes => &["<wps:wsp"],
            Issue::PictureEffects => &["<a14:imgEffect", "<a14:imgLayer"],
            Issue::TableAltText => &["<w:tblCaption", "<w:tblDescription"],
            Issue::RelativeSizes => &[
                "<wp14:sizeRelH",
                "<wp14:sizeRelV",
                "<wp14:pctPosHOffset",
                "<wp14:pctPosVOffset",
            ],
            Issue::Equations => &["<m:oMath>", "<m:oMath "],
            Issue::ContentControls => &["<w:sdt>", "<w:sdt "],
            Issue::SmartArt => &["<dgm:relIds"],
            Issue::Charts => &["<c:chart "],
            Issue::TrackedMoves => &["<w:moveFrom ", "<w:moveTo "],
            Issue::Themes => &["w:asciiTheme=", "w:themeColor="],
        }
    }

    pub const ALL: [Issue; 18] = [
        Issue::CommentReplies,
        Issue::RepeatingSections,
        Issue::ContentControlAppearance,
        Issue::OnlineVideos,
        Issue::CollapsedHeadings,
        Issue::TextEffects,
        Issue::OpenTypeFeatures,
        Issue::CheckBoxes,
        Issue::Shapes,
        Issue::PictureEffects,
        Issue::TableAltText,
        Issue::RelativeSizes,
        Issue::Equations,
        Issue::ContentControls,
        Issue::SmartArt,
        Issue::Charts,
        Issue::TrackedMoves,
        Issue::Themes,
    ];

    /// The key of the message the checker shows (`compat.issue.<key>`).
    pub fn key(self) -> &'static str {
        match self {
            Issue::CommentReplies => "compat.issue.replies",
            Issue::RepeatingSections => "compat.issue.repeating",
            Issue::ContentControlAppearance => "compat.issue.control_appearance",
            Issue::OnlineVideos => "compat.issue.video",
            Issue::CollapsedHeadings => "compat.issue.collapsed",
            Issue::TextEffects => "compat.issue.text_effects",
            Issue::OpenTypeFeatures => "compat.issue.opentype",
            Issue::CheckBoxes => "compat.issue.checkboxes",
            Issue::Shapes => "compat.issue.shapes",
            Issue::PictureEffects => "compat.issue.picture_effects",
            Issue::TableAltText => "compat.issue.table_alt",
            Issue::RelativeSizes => "compat.issue.relative",
            Issue::Equations => "compat.issue.equations",
            Issue::ContentControls => "compat.issue.controls",
            Issue::SmartArt => "compat.issue.smartart",
            Issue::Charts => "compat.issue.charts",
            Issue::TrackedMoves => "compat.issue.moves",
            Issue::Themes => "compat.issue.themes",
        }
    }

    /// Whether `version` loses this.
    pub fn affects(self, version: WordVersion) -> bool {
        version.mode() <= self.newest_affected().mode()
    }
}

/// What the checker found: each issue and how many times it occurs.
pub fn check(docx: &[u8], version: WordVersion) -> Vec<(Issue, usize)> {
    let Ok(package) = OpcPackage::open(docx) else {
        return Vec::new();
    };
    let parts: Vec<String> = package
        .part_names()
        .iter()
        .filter(|name| {
            name.starts_with("word/")
                && name.ends_with(".xml")
                && !name.contains("/theme/")
                && !name.ends_with("styles.xml")
                && !name.ends_with("fontTable.xml")
                && !name.ends_with("settings.xml")
                && !name.ends_with("numbering.xml")
                && !name.ends_with("webSettings.xml")
        })
        .filter_map(|name| package.part_str(name).ok().flatten())
        .collect();
    let has_charts = package
        .part_names()
        .iter()
        .any(|name| name.starts_with("word/charts/"));
    let mut out = Vec::new();
    for issue in Issue::ALL {
        if !issue.affects(version) {
            continue;
        }
        let mut count: usize = parts
            .iter()
            .map(|xml| {
                issue
                    .markers()
                    .iter()
                    .map(|marker| xml.matches(marker).count())
                    .sum::<usize>()
            })
            .sum();
        if issue == Issue::Charts && count == 0 && has_charts {
            count = package
                .part_names()
                .iter()
                .filter(|name| name.starts_with("word/charts/chart"))
                .count();
        }
        if count > 0 {
            out.push((issue, count));
        }
    }
    out
}

/// The layout options Word's 互換性オプション lists that the layout here
/// follows, by their element in `w:compat`.
pub const LAYOUT_OPTIONS: [&str; 4] = [
    // 東アジア言語の文字幅の調整をしない、は逆: on lays out by the line grid.
    "useFELayout",
    // 半角文字と全角文字の幅を揃える.
    "balanceSingleByteDoubleByteWidth",
    // ぶら下げインデントにタブを追加しない.
    "noTabHangInd",
    // 表の行の高さをグリッドに合わせる.
    "adjustLineHeightInTable",
];

/// Whether each of [`LAYOUT_OPTIONS`] is on in `docx`.
pub fn layout_options(docx: &[u8]) -> Vec<(&'static str, bool)> {
    let settings = OpcPackage::open(docx)
        .ok()
        .and_then(|package| package.part_str("word/settings.xml").ok().flatten())
        .unwrap_or_default();
    let compat = settings
        .find("<w:compat>")
        .and_then(|start| Some(&settings[start..start + settings[start..].find("</w:compat>")?]))
        .unwrap_or("");
    LAYOUT_OPTIONS
        .iter()
        .map(|name| {
            let tag = format!("<w:{name}");
            let on = compat.find(&tag).is_some_and(|at| {
                let element = &compat[at..at + compat[at..].find('>').unwrap_or(0)];
                !(element.contains("w:val=\"0\"")
                    || element.contains("w:val=\"false\"")
                    || element.contains("w:val=\"off\""))
            });
            (*name, on)
        })
        .collect()
}

/// `docx` with the layout option `name` turned on or off in its
/// `w:compat`, the compatibility settings kept last as the schema wants.
pub fn set_layout_option(docx: &[u8], name: &str, on: bool) -> Result<Vec<u8>, String> {
    let mut package = OpcPackage::open(docx).map_err(|error| error.to_string())?;
    let settings = package
        .part_str("word/settings.xml")
        .map_err(|error| error.to_string())?
        .ok_or("the document has no settings")?;
    let (start, end) = match settings.find("<w:compat>") {
        Some(start) => (
            start + "<w:compat>".len(),
            start
                + settings[start..]
                    .find("</w:compat>")
                    .ok_or("the settings are damaged")?,
        ),
        None => {
            let at = settings
                .rfind("</w:settings>")
                .ok_or("the settings are damaged")?;
            let settings = format!(
                "{}<w:compat></w:compat>{}",
                &settings[..at],
                &settings[at..]
            );
            package.replace("word/settings.xml", settings.into_bytes());
            let bytes = package.write().map_err(|error| error.to_string())?;
            return set_layout_option(&bytes, name, on);
        }
    };
    let compat = &settings[start..end];
    // The option goes, then comes back in front of the compatSettings if on.
    let tag = format!("<w:{name}");
    let mut without = compat.to_string();
    if let Some(at) = without.find(&tag) {
        let close = without[at..]
            .find("/>")
            .map(|end| at + end + 2)
            .unwrap_or(without.len());
        without.replace_range(at..close, "");
    }
    if on {
        let insert = without.find("<w:compatSetting").unwrap_or(without.len());
        without.insert_str(insert, &format!("<w:{name}/>"));
    }
    let changed = format!("{}{without}{}", &settings[..start], &settings[end..]);
    package.replace("word/settings.xml", changed.into_bytes());
    package.write().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::word::new::{blank_document, Blank};

    fn with_settings(compat: &str) -> Vec<u8> {
        let blank = blank_document(Blank::English);
        let mut package = OpcPackage::open(&blank).unwrap();
        let settings = package.part_str("word/settings.xml").unwrap().unwrap();
        let start = settings.find("<w:compat>").unwrap();
        let end = settings.find("</w:compat>").unwrap() + "</w:compat>".len();
        let changed = format!("{}{compat}{}", &settings[..start], &settings[end..]);
        package.replace("word/settings.xml", changed.into_bytes());
        package.write().unwrap()
    }

    #[test]
    fn the_mode_is_read_from_the_settings() {
        assert_eq!(compatibility_mode(&blank_document(Blank::Japanese)), 15);
        let word2010 = with_settings(
            r#"<w:compat><w:useFELayout/><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="14"/></w:compat>"#,
        );
        assert_eq!(compatibility_mode(&word2010), 14);
        assert!(is_compatibility_mode(14));
        // No setting: Word 2007's.
        let word2007 = with_settings("<w:compat><w:useFELayout/></w:compat>");
        assert_eq!(compatibility_mode(&word2007), 12);
    }

    #[test]
    fn converting_upgrades_the_mode() {
        let word2010 = with_settings(
            r#"<w:compat><w:useFELayout/><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="14"/></w:compat>"#,
        );
        let upgraded = upgrade(&word2010).unwrap();
        assert_eq!(compatibility_mode(&upgraded), 15);
        let settings = OpcPackage::open(&upgraded)
            .unwrap()
            .part_str("word/settings.xml")
            .unwrap()
            .unwrap();
        assert!(settings.contains("enableOpenTypeFeatures"));
        assert!(!settings.contains("useFELayout"));
        assert_eq!(settings.matches("<w:compat>").count(), 1);
    }

    #[test]
    fn layout_options_turn_on_and_off() {
        let blank = blank_document(Blank::Japanese);
        assert!(layout_options(&blank).iter().all(|(_, on)| !on));
        let on = set_layout_option(&blank, "adjustLineHeightInTable", true).unwrap();
        assert!(layout_options(&on).contains(&("adjustLineHeightInTable", true)));
        let settings = OpcPackage::open(&on)
            .unwrap()
            .part_str("word/settings.xml")
            .unwrap()
            .unwrap();
        assert!(
            settings.find("<w:adjustLineHeightInTable/>").unwrap()
                < settings.find("<w:compatSetting").unwrap()
        );
        assert_eq!(compatibility_mode(&on), 15);
        let off = set_layout_option(&on, "adjustLineHeightInTable", false).unwrap();
        assert!(layout_options(&off).iter().all(|(_, on)| !on));
    }

    #[test]
    fn the_checker_lists_what_each_version_loses() {
        let blank = blank_document(Blank::English);
        let mut package = OpcPackage::open(&blank).unwrap();
        let document = package.part_str("word/document.xml").unwrap().unwrap();
        let featured = document.replace(
            "<w:body><w:p/>",
            r#"<w:body><w:p><w:r><w:rPr><w14:glow w14:rad="63500"/></w:rPr><w:t>glow</w:t></w:r><m:oMath><m:r><m:t>x</m:t></m:r></m:oMath></w:p><w:p><w:r><w:rPr><w14:ligatures w14:val="standard"/></w:rPr><w:t>fi</w:t></w:r></w:p>"#,
        );
        package.replace("word/document.xml", featured.into_bytes());
        let docx = package.write().unwrap();
        let word2010 = check(&docx, WordVersion::Word2010);
        assert!(
            word2010.is_empty(),
            "Word 2010 keeps text effects and equations"
        );
        let word2007 = check(&docx, WordVersion::Word2007);
        assert!(word2007.contains(&(Issue::TextEffects, 1)));
        assert!(word2007.contains(&(Issue::OpenTypeFeatures, 1)));
        assert!(!word2007.iter().any(|(issue, _)| *issue == Issue::Equations));
        let word2003 = check(&docx, WordVersion::Word2003);
        assert!(word2003.contains(&(Issue::Equations, 1)));
        assert!(word2003.contains(&(Issue::TextEffects, 1)));
    }
}
