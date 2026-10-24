use crate::table_style::{from_differential, theme, TablePart};
use jade_xlsx_reader::{Color, StyleTable};
use std::{collections::BTreeMap, sync::OnceLock};
#[path = "preset_offsets.rs"]
mod offsets;
static PRESETS: OnceLock<BTreeMap<String, StyleTable>> = OnceLock::new();
pub(super) fn names() -> impl Iterator<Item = &'static str> {
    offsets::OFFSETS.iter().map(|(name, _, _)| *name)
}
fn catalog() -> &'static BTreeMap<String, StyleTable> {
    PRESETS.get_or_init(|| {
        let source = include_str!("../../data/presetTableStyles.xml");
        offsets::OFFSETS
            .iter()
            .map(|(name, start, end)| {
                let body = &source[*start..*end];
                // Pivot definitions use zero-based ids. Ordinary table presets in
                // the same normative file use one-based ids; they are not loaded.
                let xml = format!("<styleSheet>{body}</styleSheet>");
                let parsed = jade_xlsx_reader::styles::parse_styles(&xml)
                    .expect("validated normative style definition");
                ((*name).to_ascii_lowercase(), parsed)
            })
            .collect()
    })
}
pub(super) fn parts(name: &str, styles: &StyleTable) -> Option<Vec<(String, TablePart, u32)>> {
    let book = catalog().get(&name.to_ascii_lowercase())?;
    let definition = book.table_styles.first()?;
    Some(
        definition
            .elements
            .iter()
            .filter_map(|e| {
                let mut part = from_differential(
                    book.differential_styles
                        .get(e.differential_style_index? as usize)?,
                );
                let resolve = |c: Color| match c {
                    Color::Theme { index, tint, .. } => theme(styles, index, tint),
                    c => c,
                };
                part.fill = part.fill.map(resolve);
                part.text_color = part.text_color.map(resolve);
                for side in [
                    &mut part.left,
                    &mut part.right,
                    &mut part.top,
                    &mut part.bottom,
                    &mut part.inside_vertical,
                    &mut part.inside_horizontal,
                ] {
                    if let Some(Some(edge)) = side {
                        edge.color = edge.color.map(resolve);
                    }
                }
                Some((e.kind.clone(), part, e.size))
            })
            .collect(),
    )
}
