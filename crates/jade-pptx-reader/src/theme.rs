//! Theme colours and fonts — the `<a:clrScheme>` most shapes actually draw
//! with, and the `<a:fontScheme>` behind `+mj-lt`/`+mn-ea`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::slide::{Fill, Outline};
use crate::xml_tree::parse_tree;

/// The scheme's named colours, as `RRGGBB` hex without the `#`, plus the
/// master's colour map that turns `bg1`/`tx1` into scheme slots.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ThemeColors {
    colors: HashMap<String, String>,
    /// `<p:clrMap>` from the slide master: `bg1 → lt1` and so on. Empty
    /// means the default map.
    #[serde(default)]
    color_map: HashMap<String, String>,
}

impl ThemeColors {
    /// Looks a scheme name up, following the master's colour map for the
    /// `bg1`/`tx1`/`bg2`/`tx2` aliases and the default map (`bg1→lt1`,
    /// `tx1→dk1`, …) when the master states none.
    pub fn get(&self, name: &str) -> Option<&str> {
        let mapped = match self.color_map.get(name) {
            Some(slot) => slot.as_str(),
            None => match name {
                "bg1" => "lt1",
                "tx1" => "dk1",
                "bg2" => "lt2",
                "tx2" => "dk2",
                other => other,
            },
        };
        self.colors.get(mapped).map(String::as_str)
    }

    pub(crate) fn replace_scheme(&mut self, scheme: Self) {
        self.colors = scheme.colors;
    }

    /// Installs the master's `<p:clrMap>`. Called by `read_deck`; exposed so
    /// tests and other readers can build a theme without a package.
    pub fn set_color_map(&mut self, map: HashMap<String, String>) {
        self.color_map = map;
    }

    /// This theme with another colour map — how a slide's `clrMapOvr`
    /// applies without touching the deck's theme.
    pub fn with_color_map(&self, map: HashMap<String, String>) -> Self {
        Self {
            colors: self.colors.clone(),
            color_map: map,
        }
    }
}

/// The theme's major (heading) and minor (body) typefaces.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ThemeFonts {
    pub major_latin: Option<String>,
    pub minor_latin: Option<String>,
    pub major_east_asian: Option<String>,
    pub minor_east_asian: Option<String>,
}

impl ThemeFonts {
    /// Resolves a `+mj-lt` / `+mn-lt` / `+mj-ea` / `+mn-ea` reference to the
    /// face it names. Any other typeface string is not a reference and
    /// resolves to `None`.
    pub fn resolve(&self, typeface: &str) -> Option<&str> {
        let face = match typeface {
            "+mj-lt" => &self.major_latin,
            "+mn-lt" => &self.minor_latin,
            "+mj-ea" => &self.major_east_asian,
            "+mn-ea" => &self.minor_east_asian,
            _ => return None,
        };
        face.as_deref().filter(|face| !face.is_empty())
    }
}

/// Parses `<a:clrScheme>` out of a theme part.
pub fn parse_theme_colors(xml: &str) -> Result<ThemeColors> {
    let root = parse_tree(xml)?;
    let mut colors = HashMap::new();
    if let Some(scheme) = root.descendant("clrScheme") {
        for slot in &scheme.children {
            for value in &slot.children {
                let hex = match value.name.as_str() {
                    "srgbClr" => value.attr("val"),
                    // A system colour's `lastClr` is what it rendered as.
                    "sysClr" => value.attr("lastClr"),
                    _ => None,
                };
                if let Some(hex) = hex {
                    colors.insert(slot.name.clone(), hex.to_string());
                }
            }
        }
    }
    Ok(ThemeColors {
        colors,
        color_map: HashMap::new(),
    })
}

/// Parses `<a:fontScheme>` out of a theme part.
pub fn parse_theme_fonts(xml: &str) -> Result<ThemeFonts> {
    let root = parse_tree(xml)?;
    let mut fonts = ThemeFonts::default();
    if let Some(scheme) = root.descendant("fontScheme") {
        let face = |group: &str, script: &str| -> Option<String> {
            scheme
                .child(group)?
                .child(script)?
                .attr("typeface")
                .filter(|face| !face.is_empty())
                .map(str::to_string)
        };
        fonts.major_latin = face("majorFont", "latin");
        fonts.minor_latin = face("minorFont", "latin");
        fonts.major_east_asian = face("majorFont", "ea");
        fonts.minor_east_asian = face("minorFont", "ea");
    }
    Ok(fonts)
}

/// Parses `<a:fmtScheme>`'s fill style lists: the three `fillStyleLst`
/// entries a `fillRef idx` 1–3 names, and the `bgFillStyleLst` entries idx
/// 1001–1003 name. Colours stay as `phClr` for the caller to substitute.
pub fn parse_theme_fill_styles(xml: &str) -> Result<(Vec<Fill>, Vec<Fill>)> {
    let root = parse_tree(xml)?;
    let styles = |name: &str| -> Vec<Fill> {
        root.descendant("fmtScheme")
            .and_then(|scheme| scheme.child(name))
            .map(|list| {
                list.children
                    .iter()
                    .filter_map(crate::slide::parse_fill_element)
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok((styles("fillStyleLst"), styles("bgFillStyleLst")))
}

/// Theme line matrix entries remain in document order; phClr is unresolved.
pub fn parse_theme_line_styles(xml: &str) -> Result<Vec<Outline>> {
    let root = parse_tree(xml)?;
    Ok(root
        .descendant("fmtScheme")
        .and_then(|s| s.child("lnStyleLst"))
        .map(|list| {
            list.children
                .iter()
                .filter(|n| n.name == "ln")
                .map(crate::slide::parse_outline)
                .collect()
        })
        .unwrap_or_default())
}

/// Parses a slide master's `<p:clrMap>` attributes.
pub fn parse_color_map(xml: &str) -> Result<HashMap<String, String>> {
    let root = parse_tree(xml)?;
    Ok(root
        .child("clrMap")
        .map(|map| {
            map.attrs
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default())
}

/// The effects other than the outer shadow of each `effectStyleLst` entry.
pub fn parse_theme_effect_style_effects(xml: &str) -> Result<Vec<crate::Effects>> {
    let root = parse_tree(xml)?;
    Ok(root
        .descendant("fmtScheme")
        .and_then(|s| s.child("effectStyleLst"))
        .map(|list| {
            list.children
                .iter()
                .filter(|n| n.name == "effectStyle")
                .map(crate::slide::parse_effects)
                .collect()
        })
        .unwrap_or_default())
}

/// The theme's `effectStyleLst`, what a shape style's `effectRef idx` 1–3
/// names: each entry's outer shadow, when it has one.
pub fn parse_theme_effect_styles(xml: &str) -> Result<Vec<Option<crate::Shadow>>> {
    let root = parse_tree(xml)?;
    Ok(root
        .descendant("fmtScheme")
        .and_then(|s| s.child("effectStyleLst"))
        .map(|list| {
            list.children
                .iter()
                .filter(|n| n.name == "effectStyle")
                .map(crate::slide::parse_shadow)
                .collect()
        })
        .unwrap_or_default())
}
