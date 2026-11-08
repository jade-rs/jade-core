use jade_docx_reader::numbering::{AbstractNum, NumberingLevel, NumberingPart};
use std::collections::HashMap;
use std::hash::Hash;

/// Refresh an exclusively owned renderer snapshot without moving the large
/// `None` paragraph/run-property payloads in every numbering level.
pub(super) fn refresh(cached: &mut NumberingPart, incoming: &NumberingPart) {
    let NumberingPart {
        abstract_nums,
        numbering_instances,
        picture_bullets,
    } = incoming;
    refresh_map(&mut cached.abstract_nums, abstract_nums, refresh_abstract);
    refresh_map(
        &mut cached.numbering_instances,
        numbering_instances,
        Clone::clone_from,
    );
    refresh_map(
        &mut cached.picture_bullets,
        picture_bullets,
        Clone::clone_from,
    );
}

fn refresh_map<K: Clone + Eq + Hash, V: Clone>(
    cached: &mut HashMap<K, V>,
    incoming: &HashMap<K, V>,
    refresh_value: fn(&mut V, &V),
) {
    // Keep the source's iteration order as well as its entries. Rebuilding a
    // map by insertion can reorder ambiguous definitions relative to a clone.
    if cached.len() == incoming.len() && cached.keys().eq(incoming.keys()) {
        for (cached, incoming) in cached.values_mut().zip(incoming.values()) {
            refresh_value(cached, incoming);
        }
    } else {
        cached.clone_from(incoming);
    }
}

fn refresh_abstract(cached: &mut AbstractNum, incoming: &AbstractNum) {
    let AbstractNum { id, levels, preset } = incoming;
    cached.id.clone_from(id);
    cached.preset = *preset;
    cached.levels.truncate(levels.len());
    for (cached, incoming) in cached.levels.iter_mut().zip(levels) {
        refresh_level(cached, incoming);
    }
    cached
        .levels
        .extend_from_slice(&levels[cached.levels.len()..]);
}

fn refresh_level(cached: &mut NumberingLevel, incoming: &NumberingLevel) {
    // Exhaustive destructuring makes new fields a compile error instead of a
    // stale value in the renderer snapshot.
    let NumberingLevel {
        ilvl,
        start,
        restart,
        num_fmt,
        lvl_text,
        lvl_jc,
        suffix,
        indent,
        paragraph_style,
        paragraph_props,
        run_props,
        language,
        picture_bullet,
        is_legal,
    } = incoming;
    cached.ilvl = *ilvl;
    cached.start = *start;
    cached.restart = *restart;
    cached.num_fmt.clone_from(num_fmt);
    cached.lvl_text.clone_from(lvl_text);
    cached.lvl_jc.clone_from(lvl_jc);
    cached.suffix.clone_from(suffix);
    cached.indent.clone_from(indent);
    cached.paragraph_style.clone_from(paragraph_style);
    refresh_large_optional(&mut cached.paragraph_props, paragraph_props);
    refresh_large_optional(&mut cached.run_props, run_props);
    cached.language.clone_from(language);
    cached.picture_bullet.clone_from(picture_bullet);
    cached.is_legal = *is_legal;
}

fn refresh_large_optional<T: Clone>(cached: &mut Option<T>, incoming: &Option<T>) {
    match (cached.as_mut(), incoming.as_ref()) {
        (Some(cached), Some(incoming)) => cached.clone_from(incoming),
        (None, None) => {}
        _ => cached.clone_from(incoming),
    }
}
