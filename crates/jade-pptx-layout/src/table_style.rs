//! Which table style a table draws with, and what each of its cells takes
//! from it.
//!
//! PowerPoint's built-in styles are referenced by GUID and only written into
//! the package once a deck has been saved with them, so all 74 of the
//! gallery's styles are defined here: No Style (with and without the grid),
//! Themed Style 1 and 2, Light Style 1–3, Medium Style 1–4 and Dark Style 1
//! and 2, each plain and in every accent (Dark Style 2 pairs its accents).
//! A deck's own styles (`tableStyles.xml`) take precedence when the GUID
//! matches.

use jade_pptx_reader::{
    Color, Fill, Outline, RunProps, TablePartStyle, TableStyle, TableStyleFlags,
};

const NO_STYLE_NO_GRID: &str = "{2D5ABB26-0587-4C30-8999-92F81FD0307C}";
const NO_STYLE_TABLE_GRID: &str = "{5940675A-B579-460E-94D1-54222C63F5DA}";

/// The gallery: GUID, family, and the colour each style is drawn in — an
/// accent, or `None` for the family's plain (black and grey) member. Dark
/// Style 2 names its header's accent as the second colour.
const BUILTINS: &[(&str, Family, Option<&str>)] = &[
    ("{3C2FFA5D-87B4-456A-9821-1D502468CF0F}", Family::Themed1, Some("accent1")),
    ("{284E427A-3D55-4303-BF80-6455036E1DE7}", Family::Themed1, Some("accent2")),
    ("{69C7853C-536D-4A76-A0AE-DD22124D55A5}", Family::Themed1, Some("accent3")),
    ("{775DCB02-9BB8-47FD-8907-85C794F793BA}", Family::Themed1, Some("accent4")),
    ("{35758FB7-9AC5-4552-8A53-C91805E547FA}", Family::Themed1, Some("accent5")),
    ("{08FB837D-C827-4EFA-A057-4D05807E0F7C}", Family::Themed1, Some("accent6")),
    ("{D113A9D2-9D6B-4929-AA2D-F23B5EE8CBE7}", Family::Themed2, Some("accent1")),
    ("{18603FDC-E32A-4AB5-989C-0864C3EAD2B8}", Family::Themed2, Some("accent2")),
    ("{306799F8-075E-4A3A-A7F6-7FBC6576F1A4}", Family::Themed2, Some("accent3")),
    ("{E269D01E-BC32-4049-B463-5C60D7B0CCD2}", Family::Themed2, Some("accent4")),
    ("{327F97BB-C833-4FB7-BDE5-3F7075034690}", Family::Themed2, Some("accent5")),
    ("{638B1855-1B75-4FBE-930C-398BA8C253C6}", Family::Themed2, Some("accent6")),
    ("{9D7B26C5-4107-4FEC-AEDC-1716B250EE53}", Family::Light1, None),
    ("{3B4B98B0-60AC-42C2-AFA5-B58CD77FA1E5}", Family::Light1, Some("accent1")),
    ("{0E3FDE45-AF77-4B5C-9715-49D594BDF05E}", Family::Light1, Some("accent2")),
    ("{C083E6E3-FA7D-4D7B-A595-EF9225AFEA82}", Family::Light1, Some("accent3")),
    ("{D27102A9-8310-4765-A935-A1911B00CA55}", Family::Light1, Some("accent4")),
    ("{5FD0F851-EC5A-4D38-B0AD-8093EC10F338}", Family::Light1, Some("accent5")),
    ("{68D230F3-CF80-4859-8CE7-A43EE81993B5}", Family::Light1, Some("accent6")),
    ("{7E9639D4-E3E2-4D34-9284-5A2195B3D0D7}", Family::Light2, None),
    ("{69012ECD-51FC-41F1-AA8D-1B2483CD663E}", Family::Light2, Some("accent1")),
    ("{72833802-FEF1-4C79-8D5D-14CF1EAF98D9}", Family::Light2, Some("accent2")),
    ("{F2DE63D5-997A-4646-A377-4702673A728D}", Family::Light2, Some("accent3")),
    ("{17292A2E-F333-43FB-9621-5CBBE7FDCDCB}", Family::Light2, Some("accent4")),
    ("{5A111915-BE36-4E01-A7E5-04B1672EAD60}", Family::Light2, Some("accent5")),
    ("{912C8C85-51F0-491E-9774-3900AFEF0FD7}", Family::Light2, Some("accent6")),
    ("{616DA210-FB5B-4158-B5E0-FEB733F419BA}", Family::Light3, None),
    ("{BC89EF96-8CEA-46FF-86C4-4CE0E7609802}", Family::Light3, Some("accent1")),
    ("{5DA37D80-6434-44D0-B2F5-1FC5C5B2E6B3}", Family::Light3, Some("accent2")),
    ("{8799B23B-EC83-4686-B30A-512413B5E67A}", Family::Light3, Some("accent3")),
    ("{ED083AE6-46FA-4A59-8FB0-9F97EB10719F}", Family::Light3, Some("accent4")),
    ("{BDBED569-4797-4DF1-A0F4-6AAB3CD982D8}", Family::Light3, Some("accent5")),
    ("{E8B1032C-EA38-4F05-BA0D-38AFFFC7BED3}", Family::Light3, Some("accent6")),
    ("{793D81CF-94F2-401A-BA57-92F5A7B2D0C5}", Family::Medium1, None),
    ("{B301B821-A1FF-4177-AEE7-76D212191A09}", Family::Medium1, Some("accent1")),
    ("{9DCAF9ED-07DC-4A11-8D7F-57B35C25682E}", Family::Medium1, Some("accent2")),
    ("{1FECB4D8-DB02-4DC6-A0A2-4F2EBAE1DC90}", Family::Medium1, Some("accent3")),
    ("{1E171933-4619-4E11-9A3F-F7608DF75F80}", Family::Medium1, Some("accent4")),
    ("{FABFCF23-3B69-468F-B69F-88F6DE6A72F2}", Family::Medium1, Some("accent5")),
    ("{10A1B5D5-9B99-4C35-A422-299274C87663}", Family::Medium1, Some("accent6")),
    ("{073A0DAA-6AF3-43AB-8588-CEC1D06C72B9}", Family::Medium2, None),
    ("{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}", Family::Medium2, Some("accent1")),
    ("{21E4AEA4-8DFA-4A89-87EB-49C32662AFE0}", Family::Medium2, Some("accent2")),
    ("{F5AB1C69-6EDB-4FF4-983F-18BD219EF322}", Family::Medium2, Some("accent3")),
    ("{00A15C55-8517-42AA-B614-E9B94910E393}", Family::Medium2, Some("accent4")),
    ("{7DF18680-E054-41AD-8BC1-D1AEF772440D}", Family::Medium2, Some("accent5")),
    ("{93296810-A885-4BE3-A3E7-6D5BEEA58F35}", Family::Medium2, Some("accent6")),
    ("{8EC20E35-A176-4012-BC5E-935CFFF8708E}", Family::Medium3, None),
    ("{6E25E649-3F16-4E02-A733-19D2CDBF48F0}", Family::Medium3, Some("accent1")),
    ("{85BE263C-DBD7-4A20-BB59-AAB30ACAA65A}", Family::Medium3, Some("accent2")),
    ("{EB344D84-9AFB-497E-A393-DC336BA19D2E}", Family::Medium3, Some("accent3")),
    ("{EB9631B5-78F2-41C9-869B-9F39066F8104}", Family::Medium3, Some("accent4")),
    ("{74C1A8A3-306A-4EB7-A6B1-4F7E0EB9C5D6}", Family::Medium3, Some("accent5")),
    ("{2A488322-F2BA-4B5B-9748-0D474271808F}", Family::Medium3, Some("accent6")),
    ("{D7AC3CCA-C797-4891-BE02-D94E43425B78}", Family::Medium4, None),
    ("{69CF1AB2-1976-4502-BF36-3FF5EA218861}", Family::Medium4, Some("accent1")),
    ("{8A107856-5554-42FB-B03E-39F5DBC370BA}", Family::Medium4, Some("accent2")),
    ("{0505E3EF-67EA-436B-97B2-0124C06EBD24}", Family::Medium4, Some("accent3")),
    ("{C4B1156A-380E-4F78-BDF5-A606A8083BF9}", Family::Medium4, Some("accent4")),
    ("{22838BEF-8BB2-4498-84A7-C5851F593DF1}", Family::Medium4, Some("accent5")),
    ("{16D9F66E-5EB9-4882-86FB-DCBF35E3C3E4}", Family::Medium4, Some("accent6")),
    ("{E8034E78-7F5D-4C2E-B375-FC64B27BC917}", Family::Dark1, None),
    ("{125E5076-3810-47DD-B79F-674D7AD40C01}", Family::Dark1, Some("accent1")),
    ("{37CE84F3-28C3-443E-9E96-99CF82512B78}", Family::Dark1, Some("accent2")),
    ("{D03447BB-5D67-496B-8E87-E561075AD55C}", Family::Dark1, Some("accent3")),
    ("{E929F9F4-4A8F-4326-A1B4-22849713DDAB}", Family::Dark1, Some("accent4")),
    ("{8FD4443E-F989-4FC4-A0C8-D5A2AF1F390B}", Family::Dark1, Some("accent5")),
    ("{AF606853-7671-496A-8E4F-DF71F8EC918B}", Family::Dark1, Some("accent6")),
    ("{5202B0CA-FC54-4496-8BCA-5EF66A818D29}", Family::Dark2, None),
    ("{0660B408-B3CF-4A94-85FC-2B1E0A45F4A2}", Family::Dark2, Some("accent1")),
    ("{91EBBBCC-DAD2-459C-BE2E-F6DE35CF9A28}", Family::Dark2, Some("accent3")),
    ("{46F890A9-2807-4EBB-B81D-B2AA78EC7F39}", Family::Dark2, Some("accent5")),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Themed1,
    Themed2,
    Light1,
    Light2,
    Light3,
    Medium1,
    Medium2,
    Medium3,
    Medium4,
    Dark1,
    Dark2,
}

impl Family {
    fn name(self) -> &'static str {
        match self {
            Family::Themed1 => "Themed Style 1",
            Family::Themed2 => "Themed Style 2",
            Family::Light1 => "Light Style 1",
            Family::Light2 => "Light Style 2",
            Family::Light3 => "Light Style 3",
            Family::Medium1 => "Medium Style 1",
            Family::Medium2 => "Medium Style 2",
            Family::Medium3 => "Medium Style 3",
            Family::Medium4 => "Medium Style 4",
            Family::Dark1 => "Dark Style 1",
            Family::Dark2 => "Dark Style 2",
        }
    }
}

fn scheme(name: &str) -> Color {
    Color::Scheme(name.to_string())
}

fn adjusted(name: &str, alpha: Option<f32>, shade: Option<f32>, tint: Option<f32>) -> Color {
    Color::Adjusted {
        base: Box::new(scheme(name)),
        alpha,
        lum_mod: None,
        lum_off: None,
        shade,
        tint,
    }
}

fn tinted(name: &str, tint: f32) -> Color {
    adjusted(name, None, None, Some(tint))
}

fn shaded(name: &str, shade: f32) -> Color {
    adjusted(name, None, Some(shade), None)
}

fn translucent(name: &str, alpha: f32) -> Color {
    adjusted(name, Some(alpha), None, None)
}

fn solid(color: Color) -> Option<Fill> {
    Some(Fill::Solid(color))
}

fn line(color: Color, width_emu: i64) -> Option<Outline> {
    Some(Outline {
        color: Some(color),
        width_emu: Some(width_emu),
        ..Outline::default()
    })
}

/// A border the style states as absent, which hides a less specific one.
fn no_line() -> Option<Outline> {
    Some(Outline {
        no_fill: true,
        ..Outline::default()
    })
}

fn all_edges(color: Color, width_emu: i64) -> TablePartStyle {
    TablePartStyle {
        left: line(color.clone(), width_emu),
        right: line(color.clone(), width_emu),
        top: line(color.clone(), width_emu),
        bottom: line(color.clone(), width_emu),
        inside_h: line(color.clone(), width_emu),
        inside_v: line(color, width_emu),
        ..TablePartStyle::default()
    }
}

fn bold() -> TablePartStyle {
    TablePartStyle {
        bold: Some(true),
        ..TablePartStyle::default()
    }
}

fn filled(color: Color) -> Option<TablePartStyle> {
    Some(TablePartStyle {
        fill: solid(color),
        ..TablePartStyle::default()
    })
}

/// Builds one style of `family` in `accent` (`None`: its plain member).
fn family_style(id: &str, family: Family, accent: Option<&str>) -> TableStyle {
    // The plain members draw in text black where the accents draw in the
    // accent.
    let a = accent.unwrap_or("tx1");
    let name = match (family, accent) {
        (Family::Dark2, Some(first)) => {
            let n: u32 = first.trim_start_matches("accent").parse().unwrap_or(1);
            format!("{} - Accent {}/Accent {}", family.name(), n, n + 1)
        }
        (_, Some(accent)) => format!(
            "{} - Accent {}",
            family.name(),
            accent.trim_start_matches("accent")
        ),
        (_, None) => family.name().to_string(),
    };
    let mut style = TableStyle {
        id: id.to_string(),
        name: Some(name),
        ..TableStyle::default()
    };
    let text = |color: &str| TablePartStyle {
        text_color: Some(scheme(color)),
        ..TablePartStyle::default()
    };
    match family {
        Family::Themed1 => {
            // A thin accent grid over the slide, the header in the theme's
            // fill of the accent, bands washed with it.
            style.whole = Some(TablePartStyle {
                fill: Some(Fill::None),
                ..TablePartStyle {
                    text_color: Some(scheme("dk1")),
                    ..all_edges(scheme(a), 12_700)
                }
            });
            style.band1_h = filled(translucent(a, 0.4));
            style.band1_v = filled(translucent(a, 0.4));
            style.first_row = Some(TablePartStyle {
                fill: solid(scheme(a)),
                text_color: Some(scheme("lt1")),
                ..bold()
            });
            style.last_row = Some(TablePartStyle {
                top: line(scheme(a), 50_800),
                ..bold()
            });
            style.first_col = Some(bold());
            style.last_col = Some(bold());
        }
        Family::Themed2 => {
            // Solid accent with light text, framed in a lighter accent and
            // ruled in white.
            style.whole = Some(TablePartStyle {
                fill: solid(scheme(a)),
                text_color: Some(scheme("lt1")),
                left: line(tinted(a, 0.5), 12_700),
                right: line(tinted(a, 0.5), 12_700),
                top: line(tinted(a, 0.5), 12_700),
                bottom: line(tinted(a, 0.5), 12_700),
                inside_h: no_line(),
                inside_v: no_line(),
                ..TablePartStyle::default()
            });
            // A fifth of white over the accent: the accent, lightened.
            style.band1_h = filled(tinted(a, 0.8));
            style.band1_v = filled(tinted(a, 0.8));
            style.first_row = Some(TablePartStyle {
                bottom: line(scheme("lt1"), 25_400),
                ..bold()
            });
            style.last_row = Some(TablePartStyle {
                top: line(scheme("lt1"), 25_400),
                ..bold()
            });
            style.first_col = Some(TablePartStyle {
                right: line(scheme("lt1"), 25_400),
                ..bold()
            });
            style.last_col = Some(TablePartStyle {
                left: line(scheme("lt1"), 25_400),
                ..bold()
            });
        }
        Family::Light1 => {
            // Rules above and below, banded rows washed in the colour.
            style.whole = Some(TablePartStyle {
                fill: Some(Fill::None),
                top: line(scheme(a), 12_700),
                bottom: line(scheme(a), 12_700),
                left: no_line(),
                right: no_line(),
                inside_h: no_line(),
                inside_v: no_line(),
                ..text("tx1")
            });
            style.band1_h = filled(translucent(a, 0.2));
            style.band1_v = filled(translucent(a, 0.2));
            style.first_row = Some(TablePartStyle {
                bottom: line(scheme(a), 12_700),
                ..bold()
            });
            style.last_row = Some(TablePartStyle {
                top: line(scheme(a), 12_700),
                ..bold()
            });
            style.first_col = Some(bold());
            style.last_col = Some(bold());
        }
        Family::Light2 => {
            // An outline in the colour, bands ruled off in it, the header
            // solid with light text.
            style.whole = Some(TablePartStyle {
                fill: Some(Fill::None),
                left: line(scheme(a), 12_700),
                right: line(scheme(a), 12_700),
                top: line(scheme(a), 12_700),
                bottom: line(scheme(a), 12_700),
                inside_h: no_line(),
                inside_v: no_line(),
                ..text("tx1")
            });
            style.band1_h = Some(TablePartStyle {
                top: line(scheme(a), 12_700),
                bottom: line(scheme(a), 12_700),
                ..TablePartStyle::default()
            });
            let columns = TablePartStyle {
                left: line(scheme(a), 12_700),
                right: line(scheme(a), 12_700),
                ..TablePartStyle::default()
            };
            style.band1_v = Some(columns.clone());
            style.band2_v = Some(columns);
            style.first_row = Some(TablePartStyle {
                fill: solid(scheme(a)),
                text_color: Some(scheme("bg1")),
                ..bold()
            });
            style.last_row = Some(TablePartStyle {
                top: line(scheme(a), 50_800),
                ..bold()
            });
            style.first_col = Some(bold());
            style.last_col = Some(bold());
        }
        Family::Light3 => {
            // A full grid in the colour, header text in it.
            style.whole = Some(TablePartStyle {
                fill: Some(Fill::None),
                ..TablePartStyle {
                    text_color: Some(scheme("tx1")),
                    ..all_edges(scheme(a), 12_700)
                }
            });
            style.band1_h = filled(translucent(a, 0.2));
            style.band1_v = filled(translucent(a, 0.2));
            style.first_row = Some(TablePartStyle {
                text_color: Some(scheme(a)),
                bottom: line(scheme(a), 25_400),
                ..bold()
            });
            style.last_row = Some(TablePartStyle {
                top: line(scheme(a), 12_700),
                ..bold()
            });
            style.first_col = Some(bold());
            style.last_col = Some(bold());
        }
        Family::Medium1 => {
            let a = accent.unwrap_or("dk1");
            style.whole = Some(TablePartStyle {
                fill: solid(scheme("lt1")),
                text_color: Some(scheme("dk1")),
                left: line(scheme(a), 12_700),
                right: line(scheme(a), 12_700),
                top: line(scheme(a), 12_700),
                bottom: line(scheme(a), 12_700),
                inside_h: line(scheme(a), 12_700),
                inside_v: no_line(),
                ..TablePartStyle::default()
            });
            style.band1_h = filled(tinted(a, 0.2));
            style.band1_v = filled(tinted(a, 0.2));
            style.first_row = Some(TablePartStyle {
                fill: solid(scheme(a)),
                text_color: Some(scheme("lt1")),
                ..bold()
            });
            style.last_row = Some(TablePartStyle {
                fill: solid(scheme("lt1")),
                top: line(scheme(a), 50_800),
                ..bold()
            });
            style.first_col = Some(bold());
            style.last_col = Some(bold());
        }
        Family::Medium2 => {
            // A solid header, a tinted body, a white grid.
            let a = accent.unwrap_or("dk1");
            let emphasised = TablePartStyle {
                fill: solid(scheme(a)),
                text_color: Some(scheme("lt1")),
                ..bold()
            };
            style.whole = Some(TablePartStyle {
                fill: solid(tinted(a, 0.2)),
                text_color: Some(scheme("dk1")),
                ..all_edges(scheme("lt1"), 12_700)
            });
            style.first_row = Some(TablePartStyle {
                bottom: line(scheme("lt1"), 38_100),
                ..emphasised.clone()
            });
            style.last_row = Some(TablePartStyle {
                top: line(scheme("lt1"), 38_100),
                ..emphasised.clone()
            });
            style.first_col = Some(emphasised.clone());
            style.last_col = Some(emphasised);
            style.band1_h = filled(tinted(a, 0.4));
            style.band1_v = filled(tinted(a, 0.4));
        }
        Family::Medium3 => {
            // Dark rules above and below, grey bands, an accent header.
            let a = accent.unwrap_or("dk1");
            style.whole = Some(TablePartStyle {
                fill: solid(scheme("lt1")),
                text_color: Some(scheme("dk1")),
                top: line(scheme("dk1"), 25_400),
                bottom: line(scheme("dk1"), 25_400),
                left: no_line(),
                right: no_line(),
                inside_h: no_line(),
                inside_v: no_line(),
                ..TablePartStyle::default()
            });
            style.band1_h = filled(tinted("dk1", 0.2));
            style.band1_v = filled(tinted("dk1", 0.2));
            let emphasised = TablePartStyle {
                fill: solid(scheme(a)),
                text_color: Some(scheme("lt1")),
                ..bold()
            };
            style.first_row = Some(TablePartStyle {
                bottom: line(scheme("dk1"), 25_400),
                ..emphasised.clone()
            });
            style.last_row = Some(TablePartStyle {
                fill: solid(scheme("lt1")),
                top: line(scheme("dk1"), 25_400),
                ..bold()
            });
            style.first_col = Some(emphasised.clone());
            style.last_col = Some(emphasised);
        }
        Family::Medium4 => {
            // A tinted body inside a grid of the colour.
            let a = accent.unwrap_or("dk1");
            style.whole = Some(TablePartStyle {
                fill: solid(tinted(a, 0.2)),
                text_color: Some(scheme("dk1")),
                ..all_edges(scheme(a), 12_700)
            });
            style.band1_h = filled(tinted(a, 0.4));
            style.band1_v = filled(tinted(a, 0.4));
            style.first_row = Some(TablePartStyle {
                fill: solid(tinted(a, 0.2)),
                ..bold()
            });
            style.last_row = Some(TablePartStyle {
                fill: solid(tinted(a, 0.2)),
                top: line(scheme(a), 25_400),
                ..bold()
            });
            style.first_col = Some(bold());
            style.last_col = Some(bold());
        }
        Family::Dark1 => {
            // Dark cells with light text, a black header, light rules
            // setting off the emphasised parts.
            let (body, band, side) = match accent {
                Some(a) => (shaded(a, 0.75), shaded(a, 0.6), shaded(a, 0.5)),
                None => (tinted("dk1", 0.75), tinted("dk1", 0.6), tinted("dk1", 0.9)),
            };
            style.whole = Some(TablePartStyle {
                fill: solid(body.clone()),
                text_color: Some(scheme("lt1")),
                left: no_line(),
                right: no_line(),
                top: no_line(),
                bottom: no_line(),
                inside_h: no_line(),
                inside_v: no_line(),
                ..TablePartStyle::default()
            });
            style.band1_h = filled(band.clone());
            style.band1_v = filled(band);
            style.first_row = Some(TablePartStyle {
                fill: solid(scheme("dk1")),
                bottom: line(scheme("lt1"), 38_100),
                ..bold()
            });
            style.last_row = Some(TablePartStyle {
                fill: solid(body),
                top: line(scheme("lt1"), 38_100),
                ..bold()
            });
            style.first_col = Some(TablePartStyle {
                fill: solid(side.clone()),
                right: line(scheme("lt1"), 38_100),
                ..bold()
            });
            style.last_col = Some(TablePartStyle {
                fill: solid(side),
                left: line(scheme("lt1"), 38_100),
                ..bold()
            });
        }
        Family::Dark2 => {
            // A tinted body under a header in the pair's second accent.
            let (a, header) = match accent {
                Some(first) => {
                    let n: u32 = first.trim_start_matches("accent").parse().unwrap_or(1);
                    (first.to_string(), format!("accent{}", n + 1))
                }
                None => ("dk1".to_string(), "dk1".to_string()),
            };
            style.whole = Some(TablePartStyle {
                fill: solid(tinted(&a, 0.2)),
                text_color: Some(scheme("dk1")),
                left: no_line(),
                right: no_line(),
                top: no_line(),
                bottom: no_line(),
                inside_h: no_line(),
                inside_v: no_line(),
                ..TablePartStyle::default()
            });
            style.band1_h = filled(tinted(&a, 0.4));
            style.band1_v = filled(tinted(&a, 0.4));
            style.first_row = Some(TablePartStyle {
                fill: solid(scheme(&header)),
                text_color: Some(scheme("lt1")),
                ..bold()
            });
            style.last_row = Some(TablePartStyle {
                fill: solid(tinted(&a, 0.2)),
                top: line(scheme("dk1"), 50_800),
                ..bold()
            });
            style.first_col = Some(bold());
            style.last_col = Some(bold());
        }
    }
    style
}

/// A built-in style by GUID, when it is one of PowerPoint's.
pub fn builtin_table_style(id: &str) -> Option<TableStyle> {
    let id_upper = id.trim().to_ascii_uppercase();
    match id_upper.as_str() {
        NO_STYLE_NO_GRID => {
            return Some(TableStyle {
                id: id.to_string(),
                name: Some("No Style, No Grid".to_string()),
                ..TableStyle::default()
            })
        }
        NO_STYLE_TABLE_GRID => {
            return Some(TableStyle {
                id: id.to_string(),
                name: Some("No Style, Table Grid".to_string()),
                whole: Some(all_edges(scheme("tx1"), 12_700)),
                ..TableStyle::default()
            })
        }
        _ => {}
    }
    BUILTINS
        .iter()
        .find(|(guid, _, _)| *guid == id_upper)
        .map(|(_, family, accent)| family_style(id, *family, *accent))
}

/// Every built-in style's GUID, in gallery order.
pub fn builtin_table_style_ids() -> impl Iterator<Item = &'static str> {
    [NO_STYLE_NO_GRID, NO_STYLE_TABLE_GRID]
        .into_iter()
        .chain(BUILTINS.iter().map(|(guid, _, _)| *guid))
}

/// The style a table draws with: the deck's own definition first, then a
/// built-in, else none.
///
/// A table that names no style is drawn as "No Style, Table Grid": thin
/// text-coloured lines on every cell edge and nothing else, as PowerPoint
/// shows it. `tableStyles.xml`'s `def` is not consulted — it is the style
/// PowerPoint gives a table when inserting one, and PowerPoint then writes
/// that style's id into the table, so a table without an id never had it.
pub fn resolve_table_style(
    flags: &TableStyleFlags,
    deck_styles: Option<&[TableStyle]>,
) -> Option<TableStyle> {
    let Some(id) = flags.style_id.as_deref() else {
        return builtin_table_style(NO_STYLE_TABLE_GRID);
    };
    deck_styles
        .and_then(|styles| styles.iter().find(|style| style.id.eq_ignore_ascii_case(id)))
        .cloned()
        .or_else(|| builtin_table_style(id))
}

/// Where a cell sits in its table, which decides which style parts apply.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CellPosition {
    pub row: usize,
    pub column: usize,
    pub row_span: usize,
    pub column_span: usize,
    pub rows: usize,
    pub columns: usize,
}

/// What a cell takes from the style: fill, its four borders, and the text
/// defaults. Each facet comes from the most specific part that states it.
#[derive(Debug, Clone, Default)]
pub(crate) struct CellStyle {
    pub fill: Option<Fill>,
    pub left: Option<Outline>,
    pub right: Option<Outline>,
    pub top: Option<Outline>,
    pub bottom: Option<Outline>,
    pub text: RunProps,
}

/// The cells a style part covers: rows `row_start..row_end`, columns
/// `col_start..col_end`. A part's `top`/`bottom`/`left`/`right` borders
/// apply on the region's edges and its `insideH`/`insideV` between the
/// cells within it — so the header row's thick bottom line is the
/// `firstRow` part's `bottom`, even though it is inside the table.
#[derive(Debug, Clone, Copy)]
struct Region {
    row_start: usize,
    row_end: usize,
    col_start: usize,
    col_end: usize,
}

pub(crate) fn cell_style(
    style: &TableStyle,
    flags: &TableStyleFlags,
    at: CellPosition,
) -> CellStyle {
    let is_first_row = flags.first_row && at.row == 0;
    let is_last_row = flags.last_row && at.row + at.row_span == at.rows;
    let is_first_col = flags.first_col && at.column == 0;
    let is_last_col = flags.last_col && at.column + at.column_span == at.columns;
    let all = Region {
        row_start: 0,
        row_end: at.rows,
        col_start: 0,
        col_end: at.columns,
    };
    let own_rows = Region {
        row_start: at.row,
        row_end: at.row + at.row_span,
        ..all
    };
    let own_cols = Region {
        col_start: at.column,
        col_end: at.column + at.column_span,
        ..all
    };

    // Most specific first. Banding counts data rows only: the header row,
    // when it is styled as one, does not shift the stripes.
    let mut parts: Vec<(&TablePartStyle, Region)> = Vec::new();
    fn pick<'a>(
        parts: &mut Vec<(&'a TablePartStyle, Region)>,
        part: &'a Option<TablePartStyle>,
        on: bool,
        region: Region,
    ) {
        if on {
            if let Some(part) = part.as_ref() {
                parts.push((part, region));
            }
        }
    }
    pick(
        &mut parts,
        &style.first_row,
        is_first_row,
        Region {
            row_start: 0,
            row_end: 1,
            ..all
        },
    );
    pick(
        &mut parts,
        &style.last_row,
        is_last_row,
        Region {
            row_start: at.rows.saturating_sub(1),
            ..all
        },
    );
    pick(
        &mut parts,
        &style.first_col,
        is_first_col,
        Region {
            col_start: 0,
            col_end: 1,
            ..all
        },
    );
    pick(
        &mut parts,
        &style.last_col,
        is_last_col,
        Region {
            col_start: at.columns.saturating_sub(1),
            ..all
        },
    );
    if flags.band_rows && !is_first_row && !is_last_row {
        let data_row = at.row - usize::from(flags.first_row);
        if data_row.is_multiple_of(2) {
            pick(&mut parts, &style.band1_h, true, own_rows);
        } else {
            pick(&mut parts, &style.band2_h, true, own_rows);
        }
    }
    if flags.band_cols && !is_first_col && !is_last_col {
        let data_col = at.column - usize::from(flags.first_col);
        if data_col.is_multiple_of(2) {
            pick(&mut parts, &style.band1_v, true, own_cols);
        } else {
            pick(&mut parts, &style.band2_v, true, own_cols);
        }
    }
    pick(&mut parts, &style.whole, true, all);

    let edge = |on_boundary: fn(&Region, &CellPosition) -> bool,
                outer: fn(&TablePartStyle) -> Option<&Outline>,
                inside: fn(&TablePartStyle) -> Option<&Outline>|
     -> Option<Outline> {
        parts
            .iter()
            .find_map(|(part, region)| {
                if on_boundary(region, &at) {
                    outer(part)
                } else {
                    inside(part)
                }
            })
            .cloned()
    };
    CellStyle {
        fill: parts.iter().find_map(|(part, _)| part.fill.clone()),
        left: edge(
            |region, at| at.column == region.col_start,
            |part| part.left.as_ref(),
            |part| part.inside_v.as_ref(),
        ),
        right: edge(
            |region, at| at.column + at.column_span == region.col_end,
            |part| part.right.as_ref(),
            |part| part.inside_v.as_ref(),
        ),
        top: edge(
            |region, at| at.row == region.row_start,
            |part| part.top.as_ref(),
            |part| part.inside_h.as_ref(),
        ),
        bottom: edge(
            |region, at| at.row + at.row_span == region.row_end,
            |part| part.bottom.as_ref(),
            |part| part.inside_h.as_ref(),
        ),
        text: RunProps {
            bold: parts.iter().find_map(|(part, _)| part.bold),
            italic: parts.iter().find_map(|(part, _)| part.italic),
            color: parts.iter().find_map(|(part, _)| part.text_color.clone()),
            ..RunProps::default()
        },
    }
}
