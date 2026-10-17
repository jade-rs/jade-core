use crate::{Result, WriteError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParagraphAlignment {
    Left,
    Center,
    Right,
    Justify,
    Distributed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum ParagraphBullet {
    None,
    Character {
        character: String,
        font: Option<String>,
    },
    Numbered {
        scheme: String,
        #[serde(rename = "startAt")]
        start_at: Option<u32>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "value",
    rename_all = "lowercase",
    deny_unknown_fields
)]
pub enum ParagraphSpacing {
    Percent(f64),
    Points(f64),
}
impl ParagraphSpacing {
    fn valid(self, line: bool) -> bool {
        let (value, max) = match self {
            Self::Percent(value) => (value, 13200.0),
            Self::Points(value) => (value, 1584.0),
        };
        value.is_finite() && value >= 0.0 && value <= max && (!line || self.units().1 > 0)
    }
    pub(crate) fn units(self) -> (&'static str, i64) {
        match self {
            Self::Percent(value) => ("spcPct", (value * 1000.0).round() as i64),
            Self::Points(value) => ("spcPts", (value * 100.0).round() as i64),
        }
    }
}

/// Omitted properties retain their values. Margins and spacing points are authored points.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParagraphFormat {
    pub alignment: Option<ParagraphAlignment>,
    pub bullet: Option<ParagraphBullet>,
    /// Outline level, zero-based (0–8).
    pub level: Option<u8>,
    pub margin_left_pt: Option<f64>,
    pub margin_right_pt: Option<f64>,
    pub indent_pt: Option<f64>,
    pub line_spacing: Option<ParagraphSpacing>,
    pub space_before: Option<ParagraphSpacing>,
    pub space_after: Option<ParagraphSpacing>,
}
impl ParagraphFormat {
    pub fn is_empty(&self) -> bool {
        self.alignment.is_none()
            && self.bullet.is_none()
            && self.level.is_none()
            && self.margin_left_pt.is_none()
            && self.margin_right_pt.is_none()
            && self.indent_pt.is_none()
            && self.line_spacing.is_none()
            && self.space_before.is_none()
            && self.space_after.is_none()
    }
    pub fn validate(&self) -> Result<()> {
        if self.level.is_some_and(|v| v > 8)
            || self
                .margin_left_pt
                .is_some_and(|v| !v.is_finite() || !(0.0..=4032.0).contains(&v))
            || self
                .margin_right_pt
                .is_some_and(|v| !v.is_finite() || !(0.0..=4032.0).contains(&v))
            || self
                .indent_pt
                .is_some_and(|v| !v.is_finite() || !(-4032.0..=4032.0).contains(&v))
            || self.line_spacing.is_some_and(|s| !s.valid(true))
            || [self.space_before, self.space_after]
                .into_iter()
                .flatten()
                .any(|s| !s.valid(false))
        {
            return Err(WriteError::InvalidParagraphFormat);
        }
        if let Some(bullet) = &self.bullet {
            match bullet {
                ParagraphBullet::None => {}
                ParagraphBullet::Character { character, font } => {
                    if character.chars().count() != 1
                        || character
                            .chars()
                            .any(|c| c.is_control() || matches!(c, '\u{fffe}' | '\u{ffff}'))
                    {
                        return Err(WriteError::InvalidParagraphFormat);
                    }
                    crate::TextFormat {
                        font: font.clone(),
                        ..Default::default()
                    }
                    .validate()?;
                }
                ParagraphBullet::Numbered { scheme, start_at } => {
                    if !matches!(
                        scheme.as_str(),
                        "alphaLcParenBoth"
                            | "alphaUcParenBoth"
                            | "alphaLcParenR"
                            | "alphaUcParenR"
                            | "alphaLcPeriod"
                            | "alphaUcPeriod"
                            | "arabicParenBoth"
                            | "arabicParenR"
                            | "arabicPeriod"
                            | "arabicPlain"
                            | "romanLcParenBoth"
                            | "romanUcParenBoth"
                            | "romanLcParenR"
                            | "romanUcParenR"
                            | "romanLcPeriod"
                            | "romanUcPeriod"
                            | "circleNumDbPlain"
                            | "circleNumWdBlackPlain"
                            | "circleNumWdWhitePlain"
                            | "arabicDbPeriod"
                            | "arabicDbPlain"
                            | "ea1ChsPeriod"
                            | "ea1ChsPlain"
                            | "ea1ChtPeriod"
                            | "ea1ChtPlain"
                            | "ea1JpnChsDbPeriod"
                            | "ea1JpnKorPlain"
                            | "ea1JpnKorPeriod"
                            | "arabic1Minus"
                            | "arabic2Minus"
                            | "hebrew2Minus"
                            | "thaiAlphaPeriod"
                            | "thaiAlphaParenR"
                            | "thaiAlphaParenBoth"
                            | "thaiNumPeriod"
                            | "thaiNumParenR"
                            | "thaiNumParenBoth"
                            | "hindiAlphaPeriod"
                            | "hindiNumPeriod"
                            | "hindiNumParenR"
                            | "hindiAlpha1Period"
                    ) || start_at.is_some_and(|v| !(1..=32767).contains(&v))
                    {
                        return Err(WriteError::InvalidParagraphFormat);
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParagraphFormatEdit {
    pub shape_id: u32,
    pub start: usize,
    pub end: usize,
    pub format: ParagraphFormat,
}
