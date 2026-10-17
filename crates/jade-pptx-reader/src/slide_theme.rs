//! Slide theme inheritance follows the slide's own master, then layout/slide overrides.
use crate::{
    parse_theme_colors, parse_theme_fill_styles, parse_theme_fonts, parse_theme_line_styles, Deck,
    Fill, Outline, Result, ThemeColors, ThemeFonts,
};
use jade_opc::{parse_relationships, rels_part_name, resolve_target, OpcPackage};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SlideTheme {
    pub colors: ThemeColors,
    pub fonts: ThemeFonts,
    pub fill_styles: Vec<Fill>,
    pub background_fill_styles: Vec<Fill>,
    #[serde(default)]
    pub line_styles: Vec<Outline>,
}
/// One source for rendering and authoring-state queries, with legacy deck fallback.
#[derive(Debug, Clone, Copy)]
pub struct ThemeRef<'a> {
    pub colors: &'a ThemeColors,
    pub fonts: &'a ThemeFonts,
    pub fill_styles: &'a [Fill],
    pub background_fill_styles: &'a [Fill],
    pub line_styles: &'a [Outline],
}
impl Deck {
    pub fn theme_for_slide(&self, index: usize) -> ThemeRef<'_> {
        if let Some(theme) = self
            .slides
            .get(index)
            .and_then(|slide| slide.theme.as_deref())
        {
            ThemeRef {
                colors: &theme.colors,
                fonts: &theme.fonts,
                fill_styles: &theme.fill_styles,
                background_fill_styles: &theme.background_fill_styles,
                line_styles: &theme.line_styles,
            }
        } else {
            ThemeRef {
                colors: &self.theme_colors,
                fonts: &self.theme_fonts,
                fill_styles: &self.theme_fill_styles,
                background_fill_styles: &self.theme_background_fill_styles,
                line_styles: &self.theme_line_styles,
            }
        }
    }
}
#[derive(Default)]
struct Override {
    colors: Option<ThemeColors>,
    fonts: Option<ThemeFonts>,
    fills: Option<(Vec<Fill>, Vec<Fill>)>,
    lines: Option<Vec<Outline>>,
}
impl Override {
    fn read(xml: &str) -> Result<Self> {
        let root = crate::xml_tree::parse_tree(xml)?;
        Ok(Self {
            colors: root
                .child("clrScheme")
                .map(|_| parse_theme_colors(xml))
                .transpose()?,
            fonts: root
                .child("fontScheme")
                .map(|_| parse_theme_fonts(xml))
                .transpose()?,
            lines: root
                .child("fmtScheme")
                .map(|_| parse_theme_line_styles(xml))
                .transpose()?,
            fills: root
                .child("fmtScheme")
                .map(|_| parse_theme_fill_styles(xml))
                .transpose()?,
        })
    }
    fn apply(&self, theme: &mut SlideTheme) {
        if let Some(colors) = &self.colors {
            theme.colors.replace_scheme(colors.clone());
        }
        if let Some(fonts) = &self.fonts {
            theme.fonts = fonts.clone();
        }
        if let Some(lines) = &self.lines {
            theme.line_styles = lines.clone();
        }
        if let Some((fills, background)) = &self.fills {
            theme.fill_styles = fills.clone();
            theme.background_fill_styles = background.clone();
        }
    }
}
fn relation(package: &OpcPackage, owner: &str, suffix: &str) -> Result<Option<String>> {
    let Some(xml) = package.part_str(&rels_part_name(owner))? else {
        return Ok(None);
    };
    Ok(parse_relationships(&xml)?
        .by_type_suffix(suffix)
        .into_iter()
        .find(|r| !r.external)
        .map(|r| resolve_target(owner, &r.target)))
}
type ThemeKey = (Option<String>, Option<String>, Option<String>);
#[derive(Default)]
pub(crate) struct ThemeResolver {
    masters: HashMap<String, Arc<SlideTheme>>,
    overrides: HashMap<String, Override>,
    resolved: HashMap<ThemeKey, Arc<SlideTheme>>,
}
impl ThemeResolver {
    pub(crate) fn resolve(
        &mut self,
        package: &OpcPackage,
        master: Option<&str>,
        layout: Option<&str>,
        slide: &str,
        fallback: &Arc<SlideTheme>,
    ) -> Result<Arc<SlideTheme>> {
        let layout_override = layout
            .map(|p| relation(package, p, "/themeOverride"))
            .transpose()?
            .flatten();
        let slide_override = relation(package, slide, "/themeOverride")?;
        let key = (master.map(str::to_owned), layout_override, slide_override);
        if let Some(theme) = self.resolved.get(&key) {
            return Ok(theme.clone());
        }
        let mut theme = if let Some(master) = master {
            if !self.masters.contains_key(master) {
                let (colors, fonts, fill_styles, background_fill_styles, line_styles) =
                    crate::read_master_theme(package, Some(master))?;
                self.masters.insert(
                    master.into(),
                    Arc::new(SlideTheme {
                        colors,
                        fonts,
                        fill_styles,
                        background_fill_styles,
                        line_styles,
                    }),
                );
            }
            self.masters[master].clone()
        } else {
            fallback.clone()
        };
        for part in [&key.1, &key.2].into_iter().flatten() {
            if !self.overrides.contains_key(part) {
                let value = match package.part_str(part)? {
                    Some(xml) => Override::read(&xml)?,
                    None => Override::default(),
                };
                self.overrides.insert(part.clone(), value);
            }
            self.overrides[part].apply(Arc::make_mut(&mut theme));
        }
        self.resolved.insert(key, theme.clone());
        Ok(theme)
    }
}
