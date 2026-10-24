//! Word's デザイン tab: the document theme (テーマ) and its three parts — the
//! colours (配色), the fonts (フォント) and the effects (効果) — the style sets
//! (ドキュメントの書式設定), the paragraph spacing presets (段落の間隔), and
//! 既定に設定, which makes the caret's font and paragraph format the
//! document's defaults.
//!
//! A theme lives in `word/theme/theme1.xml` and reaches every run whose font
//! or colour names a theme slot (`+見出し`, `accent1`), so each of these is a
//! package edit read back in place (see `package`): the pages show what a
//! reopened file shows.

use super::package::{edit_package, PartKind};
use super::styles::{write_paragraph_format, write_run_format, StyleFormat};
use super::xml::{Element, Node};
use super::{WordFormatting, WordSession};

const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

/// A colour scheme: the twelve theme slots, `RRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorScheme {
    pub name: &'static str,
    pub dark1: &'static str,
    pub light1: &'static str,
    pub dark2: &'static str,
    pub light2: &'static str,
    pub accents: [&'static str; 6],
    pub hyperlink: &'static str,
    pub followed: &'static str,
}

impl ColorScheme {
    /// The slots in `a:clrScheme` order.
    pub fn slots(&self) -> [(&'static str, &'static str); 12] {
        [
            ("dk1", self.dark1),
            ("lt1", self.light1),
            ("dk2", self.dark2),
            ("lt2", self.light2),
            ("accent1", self.accents[0]),
            ("accent2", self.accents[1]),
            ("accent3", self.accents[2]),
            ("accent4", self.accents[3]),
            ("accent5", self.accents[4]),
            ("accent6", self.accents[5]),
            ("hlink", self.hyperlink),
            ("folHlink", self.followed),
        ]
    }
}

/// A font scheme: the heading (major) and body (minor) fonts, Latin and
/// Japanese.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontScheme {
    pub name: &'static str,
    pub major_latin: &'static str,
    pub minor_latin: &'static str,
    pub major_east_asia: &'static str,
    pub minor_east_asia: &'static str,
}

/// An effect scheme: what the theme's three effect styles draw (shapes and
/// SmartArt that name a theme effect use them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectScheme {
    pub name: &'static str,
    /// `a:effectStyleLst`'s three entries, the inner XML of each
    /// `a:effectStyle`.
    pub styles: [&'static str; 3],
}

/// A theme: its colours, fonts and effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub name: &'static str,
    pub colors: ColorScheme,
    pub fonts: FontScheme,
    pub effects: EffectScheme,
}

const fn colors(
    name: &'static str,
    dark2: &'static str,
    light2: &'static str,
    accents: [&'static str; 6],
    hyperlink: &'static str,
    followed: &'static str,
) -> ColorScheme {
    ColorScheme {
        name,
        dark1: "000000",
        light1: "FFFFFF",
        dark2,
        light2,
        accents,
        hyperlink,
        followed,
    }
}

/// Word's 配色 list.
pub const COLOR_SCHEMES: [ColorScheme; 11] = [
    colors(
        "Office",
        "0E2841",
        "E8E8E8",
        ["156082", "E97132", "196B24", "0F9ED5", "A02B93", "4EA72E"],
        "467886",
        "96607D",
    ),
    colors(
        "Office 2013 - 2022",
        "44546A",
        "E7E6E6",
        ["4472C4", "ED7D31", "A5A5A5", "FFC000", "5B9BD5", "70AD47"],
        "0563C1",
        "954F72",
    ),
    colors(
        "Office 2007 - 2010",
        "1F497D",
        "EEECE1",
        ["4F81BD", "C0504D", "9BBB59", "8064A2", "4BACC6", "F79646"],
        "0000FF",
        "800080",
    ),
    colors(
        "Grayscale",
        "000000",
        "F8F8F8",
        ["DDDDDD", "B2B2B2", "969696", "808080", "5F5F5F", "4D4D4D"],
        "5F5F5F",
        "919191",
    ),
    colors(
        "Blue",
        "17406D",
        "DBEFF9",
        ["0F6FC6", "009DD9", "0BD0D9", "10CF9B", "7CCA62", "A5C249"],
        "F49100",
        "85DFD0",
    ),
    colors(
        "Green",
        "455F51",
        "E3DED1",
        ["549E39", "8AB833", "C0CF3A", "029676", "4AB5C4", "0989B1"],
        "6B9F25",
        "BA6906",
    ),
    colors(
        "Yellow",
        "39302A",
        "E5DEDB",
        ["FFCA08", "F8931D", "CE8D3E", "EC7016", "E64823", "9C6A6A"],
        "2998E3",
        "7F723D",
    ),
    colors(
        "Orange",
        "637052",
        "CCDDEA",
        ["E48312", "BD582C", "865640", "9B8357", "C2BC80", "94A088"],
        "2998E3",
        "8C8C8C",
    ),
    colors(
        "Red",
        "323232",
        "E5C243",
        ["A5300F", "D55816", "E19825", "B19C7D", "7F5F52", "B27D49"],
        "6B9F25",
        "B26B02",
    ),
    colors(
        "Violet",
        "373545",
        "DCD8DC",
        ["AD84C6", "8784C7", "5D739A", "6997AF", "84ACB6", "6F8183"],
        "69A020",
        "8C8C8C",
    ),
    colors(
        "Paper",
        "444D26",
        "FEFAC9",
        ["A5B592", "F3A447", "E7BC29", "D092A7", "9C85C0", "809EC2"],
        "8E58B6",
        "7F6F6F",
    ),
];

const fn fonts(
    name: &'static str,
    major_latin: &'static str,
    minor_latin: &'static str,
    major_east_asia: &'static str,
    minor_east_asia: &'static str,
) -> FontScheme {
    FontScheme {
        name,
        major_latin,
        minor_latin,
        major_east_asia,
        minor_east_asia,
    }
}

/// Word's フォント list, with the Japanese faces Japanese Word pairs each
/// with.
pub const FONT_SCHEMES: [FontScheme; 9] = [
    fonts(
        "Office",
        "Aptos Display",
        "Aptos",
        "游ゴシック Light",
        "游明朝",
    ),
    fonts(
        "Office 2013 - 2022",
        "Calibri Light",
        "Calibri",
        "游ゴシック Light",
        "游明朝",
    ),
    fonts(
        "Office 2007 - 2010",
        "Cambria",
        "Calibri",
        "ＭＳ ゴシック",
        "ＭＳ 明朝",
    ),
    fonts("Calibri", "Calibri", "Calibri", "游ゴシック", "游ゴシック"),
    fonts(
        "Arial",
        "Arial",
        "Arial",
        "ＭＳ Ｐゴシック",
        "ＭＳ Ｐゴシック",
    ),
    fonts(
        "Times New Roman",
        "Times New Roman",
        "Times New Roman",
        "ＭＳ 明朝",
        "ＭＳ 明朝",
    ),
    fonts(
        "Century Gothic",
        "Century Gothic",
        "Century Gothic",
        "游ゴシック",
        "游ゴシック",
    ),
    fonts("Georgia", "Georgia", "Georgia", "游明朝", "游明朝"),
    fonts("Meiryo", "Meiryo UI", "Meiryo UI", "メイリオ", "メイリオ"),
];

const NO_EFFECT: &str = "<a:effectLst/>";
const SOFT_SHADOW: &str = r#"<a:effectLst><a:outerShdw blurRad="57150" dist="19050" dir="5400000" algn="ctr" rotWithShape="0"><a:srgbClr val="000000"><a:alpha val="63000"/></a:srgbClr></a:outerShdw></a:effectLst>"#;
const SHADOW_23: &str = r#"<a:effectLst><a:outerShdw blurRad="40000" dist="23000" dir="5400000" rotWithShape="0"><a:srgbClr val="000000"><a:alpha val="35000"/></a:srgbClr></a:outerShdw></a:effectLst>"#;
const SHADOW_20: &str = r#"<a:effectLst><a:outerShdw blurRad="40000" dist="20000" dir="5400000" rotWithShape="0"><a:srgbClr val="000000"><a:alpha val="38000"/></a:srgbClr></a:outerShdw></a:effectLst>"#;
const GLOW: &str = r#"<a:effectLst><a:glow rad="63500"><a:schemeClr val="phClr"><a:alpha val="40000"/></a:schemeClr></a:glow></a:effectLst>"#;
const REFLECTION: &str = r#"<a:effectLst><a:reflection blurRad="6350" stA="50000" endA="300" endPos="55000" dir="5400000" sy="-100000" algn="bl" rotWithShape="0"/></a:effectLst>"#;

/// Word's 効果 list.
pub const EFFECT_SCHEMES: [EffectScheme; 5] = [
    EffectScheme {
        name: "Office",
        styles: [NO_EFFECT, NO_EFFECT, SOFT_SHADOW],
    },
    EffectScheme {
        name: "Office 2007 - 2010",
        styles: [SHADOW_20, SHADOW_23, SHADOW_23],
    },
    EffectScheme {
        name: "Subtle Solids",
        styles: [NO_EFFECT, NO_EFFECT, NO_EFFECT],
    },
    EffectScheme {
        name: "Glow Edge",
        styles: [NO_EFFECT, GLOW, GLOW],
    },
    EffectScheme {
        name: "Reflection",
        styles: [NO_EFFECT, NO_EFFECT, REFLECTION],
    },
];

/// Word's テーマ list: each a colour, font and effect scheme.
pub const THEMES: [Theme; 5] = [
    Theme {
        name: "Office",
        colors: COLOR_SCHEMES[0],
        fonts: FONT_SCHEMES[0],
        effects: EFFECT_SCHEMES[0],
    },
    Theme {
        name: "Office 2013 - 2022",
        colors: COLOR_SCHEMES[1],
        fonts: FONT_SCHEMES[1],
        effects: EFFECT_SCHEMES[0],
    },
    Theme {
        name: "Office 2007 - 2010",
        colors: COLOR_SCHEMES[2],
        fonts: FONT_SCHEMES[2],
        effects: EFFECT_SCHEMES[1],
    },
    Theme {
        name: "Facet",
        colors: colors(
            "Facet",
            "2C3C43",
            "EBEBEB",
            ["90C226", "54A021", "E6B91E", "E76618", "C42F1A", "918655"],
            "99CA3C",
            "B9D181",
        ),
        fonts: fonts(
            "Facet",
            "Trebuchet MS",
            "Trebuchet MS",
            "游ゴシック",
            "游ゴシック",
        ),
        effects: EFFECT_SCHEMES[2],
    },
    Theme {
        name: "Integral",
        colors: colors(
            "Integral",
            "335B74",
            "DFE3E5",
            ["1CADE4", "2683C6", "27CED7", "42BA97", "3E8853", "62A39F"],
            "6B9F25",
            "B26B02",
        ),
        fonts: fonts(
            "Integral",
            "Tw Cen MT Condensed",
            "Tw Cen MT",
            "游ゴシック",
            "游ゴシック",
        ),
        effects: EFFECT_SCHEMES[2],
    },
];

/// The theme of a document that has none: Word's Office theme as of
/// Office 2013 – 2022, which most documents name.
pub const OFFICE_THEME: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme"><a:themeElements><a:clrScheme name="Office"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2><a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2><a:accent3><a:srgbClr val="A5A5A5"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4><a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink></a:clrScheme><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface=""/><a:cs typeface=""/><a:font script="Jpan" typeface="游ゴシック Light"/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/><a:font script="Jpan" typeface="游明朝"/></a:minorFont></a:fontScheme><a:fmtScheme name="Office"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:gradFill rotWithShape="1"><a:gsLst><a:gs pos="0"><a:schemeClr val="phClr"><a:lumMod val="110000"/><a:satMod val="105000"/><a:tint val="67000"/></a:schemeClr></a:gs><a:gs pos="50000"><a:schemeClr val="phClr"><a:lumMod val="105000"/><a:satMod val="103000"/><a:tint val="73000"/></a:schemeClr></a:gs><a:gs pos="100000"><a:schemeClr val="phClr"><a:lumMod val="105000"/><a:satMod val="109000"/><a:tint val="81000"/></a:schemeClr></a:gs></a:gsLst><a:lin ang="5400000" scaled="0"/></a:gradFill><a:gradFill rotWithShape="1"><a:gsLst><a:gs pos="0"><a:schemeClr val="phClr"><a:satMod val="103000"/><a:lumMod val="102000"/><a:tint val="94000"/></a:schemeClr></a:gs><a:gs pos="50000"><a:schemeClr val="phClr"><a:satMod val="110000"/><a:lumMod val="100000"/><a:shade val="100000"/></a:schemeClr></a:gs><a:gs pos="100000"><a:schemeClr val="phClr"><a:lumMod val="99000"/><a:satMod val="120000"/><a:shade val="78000"/></a:schemeClr></a:gs></a:gsLst><a:lin ang="5400000" scaled="0"/></a:gradFill></a:fillStyleLst><a:lnStyleLst><a:ln w="6350" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/><a:miter lim="800000"/></a:ln><a:ln w="12700" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/><a:miter lim="800000"/></a:ln><a:ln w="19050" cap="flat" cmpd="sng" algn="ctr"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/><a:miter lim="800000"/></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst><a:outerShdw blurRad="57150" dist="19050" dir="5400000" algn="ctr" rotWithShape="0"><a:srgbClr val="000000"><a:alpha val="63000"/></a:srgbClr></a:outerShdw></a:effectLst></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"><a:tint val="95000"/><a:satMod val="170000"/></a:schemeClr></a:solidFill><a:gradFill rotWithShape="1"><a:gsLst><a:gs pos="0"><a:schemeClr val="phClr"><a:tint val="93000"/><a:satMod val="150000"/><a:shade val="98000"/><a:lumMod val="102000"/></a:schemeClr></a:gs><a:gs pos="50000"><a:schemeClr val="phClr"><a:tint val="98000"/><a:satMod val="130000"/><a:shade val="90000"/><a:lumMod val="103000"/></a:schemeClr></a:gs><a:gs pos="100000"><a:schemeClr val="phClr"><a:shade val="63000"/><a:satMod val="120000"/></a:schemeClr></a:gs></a:gsLst><a:lin ang="5400000" scaled="0"/></a:gradFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>"#;

/// The document's theme as the デザイン tab shows it: which of each list is
/// current, by name, when one matches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DesignState {
    pub theme: Option<String>,
    pub colors: Option<String>,
    pub fonts: Option<String>,
    pub effects: Option<String>,
    /// The six accents, `RRGGBB`, for the swatches.
    pub accents: Vec<String>,
    pub major_font: Option<String>,
    pub minor_font: Option<String>,
}

/// The local name of a qualified one: `a:latin` → `latin`.
fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn find_local<'a>(element: &'a Element, name: &str) -> Option<&'a Element> {
    element.elements().find(|child| local(&child.name) == name)
}

fn find_local_mut<'a>(element: &'a mut Element, name: &str) -> Option<&'a mut Element> {
    element
        .elements_mut()
        .find(|child| local(&child.name) == name)
}

/// The prefix the theme uses for DrawingML (`a` almost always).
fn prefix(root: &Element) -> String {
    root.name
        .split_once(':')
        .map(|(prefix, _)| format!("{prefix}:"))
        .unwrap_or_default()
}

fn theme_elements(root: &mut Element) -> Result<&mut Element, String> {
    find_local_mut(root, "themeElements").ok_or_else(|| "the theme has no elements".to_string())
}

/// Writes `scheme` into the theme's `a:clrScheme`.
pub fn write_color_scheme(root: &mut Element, scheme: &ColorScheme) -> Result<(), String> {
    let a = prefix(root);
    let elements = theme_elements(root)?;
    let clr = find_local_mut(elements, "clrScheme").ok_or("the theme has no colour scheme")?;
    clr.set_attr("name", scheme.name);
    clr.children = scheme
        .slots()
        .into_iter()
        .map(|(slot, value)| {
            let colour = match slot {
                "dk1" => Element::new(&format!("{a}sysClr"))
                    .attr("val", "windowText")
                    .attr("lastClr", value),
                "lt1" => Element::new(&format!("{a}sysClr"))
                    .attr("val", "window")
                    .attr("lastClr", value),
                _ => Element::new(&format!("{a}srgbClr")).attr("val", value),
            };
            Node::Element(Element::new(&format!("{a}{slot}")).child(colour))
        })
        .collect();
    Ok(())
}

/// Writes `scheme` into the theme's `a:fontScheme`, the Japanese faces as
/// the `Jpan` script fonts Japanese Word reads.
pub fn write_font_scheme(root: &mut Element, scheme: &FontScheme) -> Result<(), String> {
    let a = prefix(root);
    let elements = theme_elements(root)?;
    let fonts = find_local_mut(elements, "fontScheme").ok_or("the theme has no font scheme")?;
    fonts.set_attr("name", scheme.name);
    for (which, latin, east_asia) in [
        ("majorFont", scheme.major_latin, scheme.major_east_asia),
        ("minorFont", scheme.minor_latin, scheme.minor_east_asia),
    ] {
        let collection = match find_local_mut(fonts, which) {
            Some(collection) => collection,
            None => {
                fonts
                    .children
                    .push(Node::Element(Element::new(&format!("{a}{which}"))));
                find_local_mut(fonts, which).expect("just added")
            }
        };
        let set = |collection: &mut Element, name: &str, typeface: &str| match find_local_mut(
            collection, name,
        ) {
            Some(element) => element.set_attr("typeface", typeface),
            None => collection.children.push(Node::Element(
                Element::new(&format!("{a}{name}")).attr("typeface", typeface),
            )),
        };
        set(collection, "latin", latin);
        if find_local(collection, "ea").is_none() {
            set(collection, "ea", "");
        }
        if find_local(collection, "cs").is_none() {
            set(collection, "cs", "");
        }
        let japanese = collection.elements_mut().find(|font| {
            local(&font.name) == "font" && font.get("script").as_deref() == Some("Jpan")
        });
        if let Some(font) = japanese {
            font.set_attr("typeface", east_asia);
        } else {
            collection.children.push(Node::Element(
                Element::new(&format!("{a}font"))
                    .attr("script", "Jpan")
                    .attr("typeface", east_asia),
            ));
        }
    }
    Ok(())
}

/// Writes `scheme` into the theme's `a:effectStyleLst`.
pub fn write_effect_scheme(root: &mut Element, scheme: &EffectScheme) -> Result<(), String> {
    let a = prefix(root);
    let elements = theme_elements(root)?;
    let format = find_local_mut(elements, "fmtScheme").ok_or("the theme has no format scheme")?;
    format.set_attr("name", scheme.name);
    let list = find_local_mut(format, "effectStyleLst").ok_or("the theme has no effects")?;
    list.children = scheme
        .styles
        .iter()
        .map(|inner| {
            let inner = inner
                .replace("<a:", &format!("<{a}"))
                .replace("</a:", &format!("</{a}"));
            let wrapped = format!(
                r#"<{a}effectStyle xmlns:{}="{A_NS}">{inner}</{a}effectStyle>"#,
                a.trim_end_matches(':')
            );
            let mut element = super::xml::Document::parse(&wrapped)
                .map(|document| document.root)
                .unwrap_or_else(|_| Element::new(&format!("{a}effectStyle")));
            element.remove_attr(&format!("xmlns:{}", a.trim_end_matches(':')));
            Node::Element(element)
        })
        .collect();
    Ok(())
}

/// Reads which schemes a theme part is.
pub fn design_of(theme_xml: &str) -> DesignState {
    let Ok(document) = super::xml::Document::parse(theme_xml) else {
        return DesignState::default();
    };
    let root = &document.root;
    let elements = find_local(root, "themeElements");
    let clr = elements.and_then(|elements| find_local(elements, "clrScheme"));
    let slot = |name: &str| {
        clr.and_then(|clr| find_local(clr, name))
            .and_then(|slot| slot.elements().next())
            .and_then(|colour| {
                colour
                    .get("val")
                    .filter(|_| local(&colour.name) == "srgbClr")
                    .or_else(|| colour.get("lastClr"))
            })
            .map(|value| value.to_ascii_uppercase())
    };
    let accents: Vec<String> = (1..=6)
        .filter_map(|index| slot(&format!("accent{index}")))
        .collect();
    let font_scheme = elements.and_then(|elements| find_local(elements, "fontScheme"));
    let face = |which: &str| {
        font_scheme
            .and_then(|fonts| find_local(fonts, which))
            .and_then(|collection| find_local(collection, "latin"))
            .and_then(|latin| latin.get("typeface"))
    };
    let major_font = face("majorFont");
    let minor_font = face("minorFont");
    let colors = COLOR_SCHEMES
        .iter()
        .find(|scheme| {
            scheme
                .accents
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>()
                == accents
                && slot("dk2").as_deref() == Some(scheme.dark2)
        })
        .map(|scheme| scheme.name.to_string());
    let fonts = FONT_SCHEMES
        .iter()
        .find(|scheme| {
            major_font.as_deref() == Some(scheme.major_latin)
                && minor_font.as_deref() == Some(scheme.minor_latin)
        })
        .map(|scheme| scheme.name.to_string());
    let effects = elements
        .and_then(|elements| find_local(elements, "fmtScheme"))
        .and_then(|format| format.get("name"))
        .filter(|name| EFFECT_SCHEMES.iter().any(|scheme| scheme.name == name));
    let theme = THEMES
        .iter()
        .find(|theme| {
            Some(theme.colors.name) == colors.as_deref()
                && Some(theme.fonts.name) == fonts.as_deref()
                && root.get("name").as_deref() == Some(theme.name)
        })
        .map(|theme| theme.name.to_string());
    DesignState {
        theme,
        colors,
        fonts,
        effects,
        accents,
        major_font,
        minor_font,
    }
}

/// A style set (ドキュメントの書式設定): what it does to Normal, the
/// headings and the title.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleSet {
    pub name: &'static str,
    /// Japanese Word's label for it.
    pub label_ja: &'static str,
    /// Normal's paragraph spacing: after (points) and line (multiple).
    pub after: f32,
    pub line: f32,
    /// Each heading's format, by level 1–3, then the title's.
    pub heading: [StyleFormat; 3],
    pub title: StyleFormat,
}

fn heading(size: f32, bold: bool, color: Option<&str>, before: f32, after: f32) -> StyleFormat {
    StyleFormat {
        size: Some(size),
        bold: Some(bold),
        color: color.map(str::to_string),
        space_before: Some(before),
        space_after: Some(after),
        ..StyleFormat::default()
    }
}

/// Word's style sets, as far as this editor offers them.
pub fn style_sets() -> Vec<StyleSet> {
    vec![
        StyleSet {
            name: "Word 2013 - 2022",
            label_ja: "既定 (Word 2013 - 2022)",
            after: 8.0,
            line: 1.08,
            heading: [
                heading(16.0, false, Some("2F5496"), 12.0, 0.0),
                heading(13.0, false, Some("2F5496"), 2.0, 0.0),
                heading(12.0, false, Some("1F3763"), 2.0, 0.0),
            ],
            title: heading(28.0, false, None, 0.0, 0.0),
        },
        StyleSet {
            name: "Basic (Simple)",
            label_ja: "基本 (シンプル)",
            after: 10.0,
            line: 1.15,
            heading: [
                heading(18.0, true, None, 18.0, 6.0),
                heading(14.0, true, None, 12.0, 4.0),
                heading(12.0, true, None, 10.0, 2.0),
            ],
            title: heading(26.0, true, None, 0.0, 12.0),
        },
        StyleSet {
            name: "Basic (Elegant)",
            label_ja: "基本 (エレガント)",
            after: 12.0,
            line: 1.25,
            heading: [
                heading(16.0, false, Some("404040"), 24.0, 8.0),
                heading(13.0, false, Some("404040"), 12.0, 6.0),
                heading(12.0, false, Some("595959"), 8.0, 4.0),
            ],
            title: heading(30.0, false, Some("262626"), 0.0, 16.0),
        },
        StyleSet {
            name: "Lines (Simple)",
            label_ja: "線 (シンプル)",
            after: 6.0,
            line: 1.0,
            heading: [
                StyleFormat {
                    bottom_border: Some(true),
                    ..heading(16.0, true, Some("1F3864"), 12.0, 6.0)
                },
                heading(14.0, true, Some("1F3864"), 10.0, 4.0),
                heading(12.0, true, Some("1F3864"), 8.0, 2.0),
            ],
            title: StyleFormat {
                bottom_border: Some(true),
                ..heading(28.0, true, Some("1F3864"), 0.0, 12.0)
            },
        },
        StyleSet {
            name: "Black & White (Classic)",
            label_ja: "白黒 (クラシック)",
            after: 6.0,
            line: 1.0,
            heading: [
                heading(16.0, true, Some("000000"), 24.0, 6.0),
                heading(13.0, true, Some("000000"), 12.0, 4.0),
                heading(12.0, false, Some("000000"), 8.0, 2.0),
            ],
            title: heading(26.0, true, Some("000000"), 0.0, 12.0),
        },
    ]
}

/// 段落の間隔: Normal's space after and line spacing.
pub const PARAGRAPH_SPACINGS: [(&str, &str, f32, f32, f32); 6] = [
    ("none", "段落間隔なし", 0.0, 0.0, 1.0),
    ("compact", "コンパクト", 0.0, 4.0, 1.0),
    ("tight", "狭く", 0.0, 6.0, 1.0),
    ("open", "広く", 0.0, 10.0, 1.15),
    ("relaxed", "ゆったり", 0.0, 6.0, 1.5),
    ("double", "2 行", 0.0, 8.0, 2.0),
];

/// The デザイン tab's commands, for every session.
pub trait WordDesign: WordSession {
    /// The theme as it stands.
    fn design_state(&self) -> DesignState {
        self.word_document()
            .related_part_xml("theme")
            .map(|xml| design_of(&xml))
            .unwrap_or_default()
    }

    /// Applies one theme: its name, colours, fonts and effects.
    fn apply_theme(&mut self, theme: &Theme) -> Result<(), String> {
        let theme = *theme;
        self.edit_theme(move |root| {
            root.set_attr("name", theme.name);
            write_color_scheme(root, &theme.colors)?;
            write_font_scheme(root, &theme.fonts)?;
            write_effect_scheme(root, &theme.effects)
        })
    }

    fn apply_color_scheme(&mut self, scheme: &ColorScheme) -> Result<(), String> {
        let scheme = *scheme;
        self.edit_theme(move |root| write_color_scheme(root, &scheme))
    }

    fn apply_font_scheme(&mut self, scheme: &FontScheme) -> Result<(), String> {
        let scheme = *scheme;
        self.edit_theme(move |root| write_font_scheme(root, &scheme))
    }

    fn apply_effect_scheme(&mut self, scheme: &EffectScheme) -> Result<(), String> {
        let scheme = *scheme;
        self.edit_theme(move |root| write_effect_scheme(root, &scheme))
    }

    fn edit_theme(
        &mut self,
        edit: impl FnOnce(&mut Element) -> Result<(), String> + 'static,
    ) -> Result<(), String> {
        edit_package(
            self,
            |package| package.edit(PartKind::Theme, edit),
            Vec::new(),
        )
    }

    /// Applies a style set: Normal's spacing and the headings' and title's
    /// look, through the styles.
    fn apply_style_set(&mut self, set: &StyleSet) -> Result<(), String> {
        let set = set.clone();
        edit_package(
            self,
            move |package| {
                package.edit(PartKind::Styles, |styles| {
                    let normal = StyleFormat {
                        space_after: Some(set.after),
                        line_spacing: Some(set.line),
                        ..StyleFormat::default()
                    };
                    super::styles::edit_builtin_style(styles, "Normal", &normal)?;
                    for (level, format) in set.heading.iter().enumerate() {
                        super::styles::edit_builtin_style(
                            styles,
                            &format!("heading {}", level + 1),
                            format,
                        )?;
                    }
                    super::styles::edit_builtin_style(styles, "Title", &set.title)
                })
            },
            Vec::new(),
        )
    }

    /// 段落の間隔: Normal's space after and line spacing, in points and as
    /// a multiple.
    fn apply_paragraph_spacing(
        &mut self,
        before: f32,
        after: f32,
        line: f32,
    ) -> Result<(), String> {
        edit_package(
            self,
            move |package| {
                package.edit(PartKind::Styles, |styles| {
                    let format = StyleFormat {
                        space_before: Some(before),
                        space_after: Some(after),
                        line_spacing: Some(line),
                        ..StyleFormat::default()
                    };
                    super::styles::edit_builtin_style(styles, "Normal", &format)
                })
            },
            Vec::new(),
        )
    }

    /// 既定に設定: the caret's font, size and paragraph spacing become the
    /// document's defaults (`w:docDefaults`), so every paragraph that sets
    /// none of its own takes them.
    fn set_as_default(&mut self) -> Result<(), String> {
        let state = WordFormatting::format_state(self);
        let at = self
            .selection_range()
            .map(|range| range.normalized().0.paragraph)
            .unwrap_or(0);
        let spacing = match super::query(
            self.word_document(),
            jade_docx_headless_api::HeadlessQuery::Paragraph { paragraph: at },
        ) {
            Some(jade_docx_headless_api::HeadlessQueryResult::Paragraph {
                value: Some(paragraph),
                ..
            }) => paragraph.spacing,
            _ => None,
        };
        let format = StyleFormat {
            font: state.font_family.clone(),
            east_asia_font: state.font_family.clone(),
            size: state.font_size,
            space_before: spacing.as_ref().and_then(|spacing| spacing.before),
            space_after: spacing.as_ref().and_then(|spacing| spacing.after),
            line_spacing: spacing
                .as_ref()
                .filter(|spacing| {
                    spacing
                        .line_rule
                        .as_deref()
                        .is_none_or(|rule| rule == "auto")
                })
                .and_then(|spacing| spacing.line),
            ..StyleFormat::default()
        };
        set_document_defaults(self, &format)
    }
}

impl<T: WordSession + ?Sized> WordDesign for T {}

/// Writes `format` into `w:docDefaults`: its run part into `w:rPrDefault`,
/// its paragraph part into `w:pPrDefault`.
pub fn set_document_defaults<S: WordSession + ?Sized>(
    session: &mut S,
    format: &StyleFormat,
) -> Result<(), String> {
    let format = format.clone();
    edit_package(
        session,
        move |package| {
            package.edit(PartKind::Styles, |styles| {
                let defaults = styles.ensure("w:docDefaults", super::styles::STYLES_ORDER);
                let run = defaults.ensure("w:rPrDefault", &["w:rPrDefault", "w:pPrDefault"]);
                let rpr = run.ensure("w:rPr", &["w:rPr"]);
                write_run_format(rpr, &format);
                let paragraph = defaults.ensure("w:pPrDefault", &["w:rPrDefault", "w:pPrDefault"]);
                let ppr = paragraph.ensure("w:pPr", &["w:pPr"]);
                write_paragraph_format(ppr, &format);
                Ok(())
            })
        },
        Vec::new(),
    )
}
