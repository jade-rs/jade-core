//! Borrowed placeholder properties shared by painting and authoring queries.
use crate::InheritedContent;
use jade_pptx_reader::{
    Autofit, Fill, Outline, Placeholder, Shape, ShapeItem, ShapeStyle, SlideContent, TextBody,
    TextVertical, Transform,
};

pub(crate) fn definition(item: &ShapeItem) -> Option<(&Placeholder, Option<Transform>)> {
    match item {
        ShapeItem::Shape(s) => s.placeholder.as_ref().map(|ph| (ph, s.transform)),
        ShapeItem::Picture(p) => p.placeholder.as_ref().map(|ph| (ph, p.transform)),
        _ => None,
    }
}
fn unique(content: &SlideContent, accept: impl Fn(&Placeholder) -> bool) -> Option<&ShapeItem> {
    let mut matches = content
        .shapes
        .iter()
        .filter(|s| definition(s).is_some_and(|(ph, _)| accept(ph)));
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}
// Slide/layout idx and layout/master type are distinct inheritance contracts.
pub(crate) fn parents<'a>(
    ph: &Placeholder,
    inherited: &InheritedContent<'a>,
) -> [Option<&'a ShapeItem>; 2] {
    if ph.index == Some(u32::MAX) {
        return [None, None];
    }
    let layout = inherited
        .layout
        .and_then(|c| unique(c, |p| p.index.unwrap_or(0) == ph.index.unwrap_or(0)));
    if inherited.layout.is_some() && layout.is_none() {
        return [None, None];
    }
    let role = layout.and_then(definition).map_or(ph, |(p, _)| p);
    let kind = crate::style::master_kind(&role.kind);
    let master = inherited.master.and_then(|c| unique(c, |p| p.kind == kind));
    [layout, master]
}
pub(crate) fn inherited_shapes<'a>(
    shape: &Shape,
    inherited: &InheritedContent<'a>,
) -> impl Iterator<Item = &'a Shape> {
    shape
        .placeholder
        .as_ref()
        .map(|ph| parents(ph, inherited))
        .unwrap_or([None, None])
        .into_iter()
        .flatten()
        .filter_map(|item| match item {
            ShapeItem::Shape(s) => Some(s),
            _ => None,
        })
}
pub(crate) struct Visual<'a> {
    pub geometry: &'a Shape,
    pub fill: Option<&'a Fill>,
    pub image: Option<&'a Shape>,
    pub outline: Option<Outline>,
    pub style: Option<&'a ShapeStyle>,
}
pub(crate) fn visual<'a>(shape: &'a Shape, inherited: &InheritedContent<'a>) -> Visual<'a> {
    let parents = inherited_shapes(shape, inherited);
    let chain: Vec<_> = std::iter::once(shape).chain(parents).collect();
    let geometry = chain
        .iter()
        .copied()
        .find(|s| s.preset_geometry.is_some() || s.custom_geometry.is_some())
        .unwrap_or(shape);
    // An explicit noFill or picture fill stops lower-priority fills.
    let paint = chain
        .iter()
        .copied()
        .find(|s| s.fill.is_some() || s.image_rel_id.is_some() || s.image_part.is_some());
    let lines: Vec<_> = chain.iter().filter_map(|s| s.outline.as_ref()).collect();
    let line_fill = lines.iter().find(|l| l.no_fill || l.color.is_some());
    let outline = (!lines.is_empty()).then(|| Outline {
        no_fill: line_fill.is_some_and(|l| l.no_fill),
        color: line_fill.and_then(|l| l.color.clone()),
        width_emu: lines.iter().find_map(|l| l.width_emu),
        dash: lines.iter().find_map(|l| l.dash.clone()),
    });
    Visual {
        geometry,
        fill: paint.and_then(|s| s.fill.as_ref()),
        image: paint.filter(|s| s.image_rel_id.is_some() || s.image_part.is_some()),
        outline,
        style: chain.iter().find_map(|s| s.style.as_ref()),
    }
}
#[derive(Default)]
pub(crate) struct Body {
    pub wrap: Option<bool>,
    pub autofit: Option<Autofit>,
    pub vertical: Option<Option<TextVertical>>,
}
impl Body {
    pub fn inherit(&mut self, text: &TextBody) {
        let p = text.body_property_presence;
        if p.is_none_or(|p| p.wrap) {
            self.wrap = self.wrap.or(Some(text.wrap));
        }
        if p.is_none_or(|p| p.autofit) {
            self.autofit = self.autofit.or(Some(text.autofit));
        }
        if p.is_none_or(|p| p.vertical) {
            self.vertical = self.vertical.or(Some(text.vertical));
        }
    }
}
