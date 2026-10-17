use crate::{slide_xml, validate_text_box, Result, TextFormat, WriteError};
use jade_pptx_reader::Transform;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShapePreset {
    Rect,
    RoundRect,
    Ellipse,
    Triangle,
    Diamond,
    Hexagon,
}
impl ShapePreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rect => "rect",
            Self::RoundRect => "roundRect",
            Self::Ellipse => "ellipse",
            Self::Triangle => "triangle",
            Self::Diamond => "diamond",
            Self::Hexagon => "hexagon",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShapeInsert {
    pub preset: ShapePreset,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub format: TextFormat,
    /// Omit for theme accent1; "none" removes the fill. Otherwise #RRGGBB.
    pub fill: Option<String>,
    /// Omit for theme accent1; "none" removes the outline. Otherwise #RRGGBB.
    pub outline: Option<String>,
    /// Defaults to one point; DrawingML widths range from zero to 1584 points.
    pub outline_width_pt: Option<f64>,
}
pub fn validate_shape_insert(transform: Transform, value: &ShapeInsert) -> Result<()> {
    validate_text_box(transform, &value.text, &value.format)?;
    for color in [&value.fill, &value.outline].into_iter().flatten() {
        if color != "none"
            && !(color.len() == 7
                && color.starts_with('#')
                && color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit))
        {
            return Err(WriteError::InvalidShapeStyle);
        }
    }
    if value
        .outline_width_pt
        .is_some_and(|w| !w.is_finite() || !(0.0..=1584.0).contains(&w))
    {
        return Err(WriteError::InvalidShapeStyle);
    }
    Ok(())
}
/// Adds an ordinary shape at the highest layer, preserving unrelated package parts.
pub fn insert_shape(
    bytes: &[u8],
    slide: usize,
    transform: Transform,
    value: &ShapeInsert,
) -> Result<(Vec<u8>, u32)> {
    validate_shape_insert(transform, value)?;
    let (mut package, part, xml) = slide_xml(bytes, slide)?;
    let (changed, id) = crate::xml::shape_insertion::insert(&xml, transform, value)?;
    package.replace(&part, changed.into_bytes());
    let result = package.write()?;
    jade_pptx_reader::read_deck(&result)?;
    Ok((result, id))
}
