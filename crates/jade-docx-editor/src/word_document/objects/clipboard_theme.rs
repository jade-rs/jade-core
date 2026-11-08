//! Resolve the source theme before a detached object encounters a destination theme.
use super::*;
use crate::drawing_xml::{get_attr, set_attr};
use crate::theme::{text_theme_color, ThemeColorScope};

pub(super) fn freeze_theme(
    element: &mut XmlElement,
    theme: &ThemePart,
    mapping: &HashMap<String, String>,
) {
    let _scope = ThemeColorScope::enter_with_mapping(Some(theme), mapping);
    let drawing_theme = theme.with_drawing_color_mapping(mapping);
    freeze_element(element, &drawing_theme);
}

fn freeze_element(element: &mut XmlElement, theme: &ThemePart) {
    if element.name == "w:rFonts" {
        for (reference, explicit) in [
            ("w:asciiTheme", "w:ascii"),
            ("w:hAnsiTheme", "w:hAnsi"),
            ("w:eastAsiaTheme", "w:eastAsia"),
            ("w:cstheme", "w:cs"),
            ("w:csTheme", "w:cs"),
        ] {
            if let Some(face) = get_attr(element, reference)
                .and_then(|key| theme.resolve_theme_font(key))
                .map(str::to_owned)
            {
                set_attr(element, explicit, Some(face));
                set_attr(element, reference, None);
            }
        }
        for name in ["w:ascii", "w:hAnsi", "w:eastAsia", "w:cs"] {
            if let Some(face) = get_attr(element, name)
                .and_then(|key| theme.resolve_theme_token(key))
                .map(str::to_owned)
            {
                set_attr(element, name, Some(face));
            }
        }
    }
    if element.name.starts_with("w:") {
        for (reference, tint, shade, explicit) in [
            (
                "w:themeColor",
                "w:themeTint",
                "w:themeShade",
                if element.name == "w:color" {
                    "w:val"
                } else {
                    "w:color"
                },
            ),
            (
                "w:themeFill",
                "w:themeFillTint",
                "w:themeFillShade",
                "w:fill",
            ),
        ] {
            let color = get_attr(element, reference).and_then(|key| {
                text_theme_color(key, get_attr(element, tint), get_attr(element, shade))
            });
            if let Some(color) = color {
                set_attr(
                    element,
                    explicit,
                    Some(color.trim_start_matches('#').to_owned()),
                );
                for name in [reference, tint, shade] {
                    set_attr(element, name, None);
                }
            }
        }
    }
    if element.name == "a:schemeClr" {
        if let Some(color) = get_attr(element, "val")
            .and_then(|key| theme.resolve_theme_color(key))
            .map(|color| color.trim_start_matches('#').to_owned())
        {
            // Retain alpha, luminance and other DrawingML transform children.
            element.name = "a:srgbClr".into();
            set_attr(element, "val", Some(color));
        }
    }
    if matches!(element.name.as_str(), "a:latin" | "a:ea" | "a:cs") {
        if let Some(face) = get_attr(element, "typeface")
            .and_then(|key| theme.resolve_theme_token(key))
            .map(str::to_owned)
        {
            set_attr(element, "typeface", Some(face));
        }
    }
    for child in &mut element.children {
        if let XmlNode::Element(child) = child {
            freeze_element(child, theme);
        }
    }
}
