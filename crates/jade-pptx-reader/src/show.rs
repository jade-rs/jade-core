//! How a deck is presented: スライド ショーの設定 (`presProps.xml`'s
//! `<p:showPr>`), 目的別スライド ショー (`presentation.xml`'s
//! `<p:custShowLst>`) and each slide's rehearsed advance time
//! (`<p:transition advTm>`).
//!
//! These are read on their own rather than as part of [`crate::read_deck`]:
//! nothing that lays out or paints a slide needs them, and a show reads them
//! once when it starts.

use jade_opc::OpcPackage;
use serde::{Deserialize, Serialize};

use crate::xml_tree::{parse_tree, XmlElement};
use crate::Result;

/// 種類 in スライド ショーの設定.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ShowType {
    /// 発表者として使用する (フル スクリーン表示) — `<p:present/>`.
    #[default]
    Speaker,
    /// 出席者として閲覧する (ウィンドウ表示) — `<p:browse>`; `scrollbar` is
    /// its `showScrollbar` (default on).
    Individual { scrollbar: bool },
    /// 自動プレゼンテーション (フル スクリーン表示) — `<p:kiosk>`, which
    /// plays on its timings, loops and ignores clicks. `restart_ms` is how
    /// long it waits without input before starting over (default 5 min).
    Kiosk { restart_ms: u32 },
}

/// スライドの表示: which slides a show plays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ShowRange {
    /// すべて — `<p:sldAll/>`.
    #[default]
    All,
    /// 開始 … 終了, 1-based and inclusive as the file states them —
    /// `<p:sldRg st end>`.
    Slides { from: usize, to: usize },
    /// 目的別スライド ショー: the custom show with this `id`.
    Custom(u32),
}

/// `<p:showPr>`, with PowerPoint's defaults where the file is silent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShowSettings {
    pub show_type: ShowType,
    /// Esc キーが押されるまで繰り返す — `loop`.
    pub loop_until_esc: bool,
    /// ナレーションを付けない — the inverse of `showNarration`.
    pub without_narration: bool,
    /// アニメーションを表示しない — the inverse of `showAnimation`.
    pub without_animation: bool,
    /// 保存済みのタイミング (`useTimings`) rather than 手動.
    pub use_timings: bool,
    pub range: ShowRange,
    /// ペンの色 (`<p:penClr>`) as `RRGGBB`, when the file names one.
    pub pen_color: Option<String>,
    /// レーザー ポインターの色 (`p14:laserClr`) as `RRGGBB`.
    pub laser_color: Option<String>,
}

impl Default for ShowSettings {
    /// What a new PowerPoint deck has: presented by a speaker, every slide,
    /// narration and animation on, timings used.
    fn default() -> Self {
        Self {
            show_type: ShowType::Speaker,
            loop_until_esc: false,
            without_narration: false,
            without_animation: false,
            use_timings: true,
            range: ShowRange::All,
            pen_color: None,
            laser_color: None,
        }
    }
}

/// One 目的別スライド ショー: a named list of the deck's slides, in the order
/// it plays them (a slide may appear more than once).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomShow {
    pub id: u32,
    pub name: String,
    /// 0-based slide indices in presentation order.
    pub slides: Vec<usize>,
}

/// How a slide hands over to the next: `<p:transition advClick advTm>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlideAdvance {
    /// クリック時 (default on).
    pub on_click: bool,
    /// 自動的に切り替え: after this many milliseconds, when set.
    pub after_ms: Option<u32>,
}

impl Default for SlideAdvance {
    fn default() -> Self {
        Self {
            on_click: true,
            after_ms: None,
        }
    }
}

/// Everything a slide show needs besides the slides themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ShowInfo {
    pub settings: ShowSettings,
    pub custom_shows: Vec<CustomShow>,
    /// One per slide, in presentation order.
    pub advances: Vec<SlideAdvance>,
}

/// Reads the show settings, the custom shows and every slide's advance.
pub fn read_show(bytes: &[u8]) -> Result<ShowInfo> {
    let package = OpcPackage::open(bytes)?;
    let presentation = crate::presentation_part_name(&package)?;
    let rels = match package.part_str(&jade_opc::rels_part_name(&presentation))? {
        Some(xml) => jade_opc::parse_relationships(&xml)?,
        None => Default::default(),
    };
    let slides = crate::read_pptx(bytes)?.slides;

    let settings = match rels
        .target_by_type_suffix("/presProps")
        .map(|target| jade_opc::resolve_target(&presentation, target))
    {
        Some(part) => match package.part_str(&part)? {
            Some(xml) => parse_show_settings(&parse_tree(&xml)?),
            None => ShowSettings::default(),
        },
        None => ShowSettings::default(),
    };

    let mut custom_shows = Vec::new();
    if let Some(xml) = package.part_str(&presentation)? {
        let root = parse_tree(&xml)?;
        if let Some(list) = root.child("custShowLst") {
            for show in list.children.iter().filter(|c| c.name == "custShow") {
                let ids: Vec<usize> = show
                    .child("sldLst")
                    .map(|list| {
                        list.children
                            .iter()
                            .filter(|c| c.name == "sld")
                            .filter_map(|sld| {
                                let target = rels.target(sld.attr("id")?)?;
                                let part = jade_opc::resolve_target(&presentation, target);
                                slides.iter().position(|slide| slide.part_name == part)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                custom_shows.push(CustomShow {
                    id: show.attr("id").and_then(|v| v.parse().ok()).unwrap_or(0),
                    name: show.attr("name").unwrap_or_default().to_string(),
                    slides: ids,
                });
            }
        }
    }

    let mut advances = Vec::with_capacity(slides.len());
    for slide in &slides {
        let advance = match package.part_str(&slide.part_name)? {
            Some(xml) => slide_advance(&parse_tree(&xml)?),
            None => SlideAdvance::default(),
        };
        advances.push(advance);
    }

    Ok(ShowInfo {
        settings,
        custom_shows,
        advances,
    })
}

fn flag(element: &XmlElement, name: &str, default: bool) -> bool {
    match element.attr(name) {
        Some("1" | "true" | "on") => true,
        Some("0" | "false" | "off") => false,
        _ => default,
    }
}

fn parse_show_settings(root: &XmlElement) -> ShowSettings {
    let Some(show) = root.child("showPr") else {
        return ShowSettings::default();
    };
    let show_type = if let Some(browse) = show.child("browse") {
        ShowType::Individual {
            scrollbar: flag(browse, "showScrollbar", true),
        }
    } else if let Some(kiosk) = show.child("kiosk") {
        ShowType::Kiosk {
            restart_ms: kiosk
                .attr("restart")
                .and_then(|v| v.parse().ok())
                .unwrap_or(300_000),
        }
    } else {
        ShowType::Speaker
    };
    let range = if let Some(range) = show.child("sldRg") {
        let from = range.attr("st").and_then(|v| v.parse().ok()).unwrap_or(1);
        let to = range
            .attr("end")
            .and_then(|v| v.parse().ok())
            .unwrap_or(from);
        ShowRange::Slides { from, to }
    } else if let Some(custom) = show.child("custShow") {
        ShowRange::Custom(custom.attr("id").and_then(|v| v.parse().ok()).unwrap_or(0))
    } else {
        ShowRange::All
    };
    ShowSettings {
        show_type,
        loop_until_esc: flag(show, "loop", false),
        without_narration: !flag(show, "showNarration", false),
        without_animation: !flag(show, "showAnimation", true),
        use_timings: flag(show, "useTimings", true),
        range,
        pen_color: show.child("penClr").and_then(simple_color),
        laser_color: show.descendant("laserClr").and_then(simple_color),
    }
}

/// The colour a `penClr`/`laserClr` names: an sRGB value or one of the
/// preset names PowerPoint writes there.
fn simple_color(container: &XmlElement) -> Option<String> {
    let color = container.children.first()?;
    match color.name.as_str() {
        "srgbClr" => color.attr("val").map(|v| v.to_ascii_uppercase()),
        "prstClr" => Some(
            match color.attr("val")? {
                "red" => "FF0000",
                "black" => "000000",
                "white" => "FFFFFF",
                "blue" => "0000FF",
                "green" => "008000",
                "lime" => "00FF00",
                "yellow" => "FFFF00",
                "cyan" | "aqua" => "00FFFF",
                "magenta" | "fuchsia" => "FF00FF",
                "orange" => "FFA500",
                "purple" => "800080",
                "gray" | "grey" => "808080",
                _ => return None,
            }
            .to_string(),
        ),
        _ => None,
    }
}

/// The slide's `<p:transition>` (inside `mc:AlternateContent` when the
/// transition is one PowerPoint 2010 added — the fallback carries the same
/// advance).
fn slide_advance(root: &XmlElement) -> SlideAdvance {
    let Some(transition) = root.descendant("transition") else {
        return SlideAdvance::default();
    };
    SlideAdvance {
        on_click: flag(transition, "advClick", true),
        after_ms: transition.attr("advTm").and_then(|v| v.parse().ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_settings_read_with_powerpoint_defaults() {
        let root = parse_tree(
            r#"<p:presentationPr xmlns:p="p" xmlns:a="a"><p:showPr loop="1" showNarration="1" useTimings="0"><p:kiosk restart="60000"/><p:sldRg st="2" end="4"/><p:penClr><a:prstClr val="red"/></p:penClr><p:extLst><p:ext uri="x"><p14:laserClr xmlns:p14="p14"><a:srgbClr val="00ff00"/></p14:laserClr></p:ext></p:extLst></p:showPr></p:presentationPr>"#,
        )
        .unwrap();
        let settings = parse_show_settings(&root);
        assert_eq!(settings.show_type, ShowType::Kiosk { restart_ms: 60000 });
        assert!(settings.loop_until_esc);
        assert!(!settings.without_narration);
        assert!(!settings.without_animation);
        assert!(!settings.use_timings);
        assert_eq!(settings.range, ShowRange::Slides { from: 2, to: 4 });
        assert_eq!(settings.pen_color.as_deref(), Some("FF0000"));
        assert_eq!(settings.laser_color.as_deref(), Some("00FF00"));

        let silent = parse_tree(r#"<p:presentationPr xmlns:p="p"/>"#).unwrap();
        assert_eq!(parse_show_settings(&silent), ShowSettings::default());
    }

    #[test]
    fn advance_is_read_from_the_transition_or_its_fallback() {
        let root = parse_tree(
            r#"<p:sld xmlns:p="p" xmlns:mc="mc"><mc:AlternateContent><mc:Choice Requires="p14"><p:transition spd="slow" advClick="0" advTm="2500"/></mc:Choice><mc:Fallback><p:transition advClick="0" advTm="2500"/></mc:Fallback></mc:AlternateContent></p:sld>"#,
        )
        .unwrap();
        assert_eq!(
            slide_advance(&root),
            SlideAdvance {
                on_click: false,
                after_ms: Some(2500)
            }
        );
        let plain = parse_tree(r#"<p:sld xmlns:p="p"/>"#).unwrap();
        assert_eq!(slide_advance(&plain), SlideAdvance::default());
    }
}
