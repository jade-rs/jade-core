//! Where a picture's `<p:pic>` (and its alternate-content twins) sit in a
//! slide, for edits that rewrite the element.

use std::ops::Range;

use super::{crop, parse_nodes};
use crate::{Result, WriteError};

/// The byte ranges of picture `id`'s `<p:pic>` elements.
pub(crate) fn picture_ranges(xml: &str, id: u32) -> Result<Vec<Range<usize>>> {
    let nodes = parse_nodes(xml)?;
    let shape = crop::selected(&nodes, id)?;
    if !shape.is(1, "pic") {
        return Err(WriteError::UnsupportedObjectOperation(id));
    }
    let ranges: Vec<_> = crop::branches(&nodes, shape, id)
        .into_iter()
        .filter(|node| node.is(1, "pic"))
        .map(|node| node.outer.clone())
        .collect();
    if ranges.is_empty() {
        return Err(WriteError::NoShape(id));
    }
    Ok(ranges)
}

/// Every picture on the slide: its id and range.
pub(crate) fn all_pictures(xml: &str) -> Result<Vec<(u32, Range<usize>)>> {
    let nodes = parse_nodes(xml)?;
    Ok(super::shape_nodes(&nodes)
        .into_iter()
        .filter(|node| node.is(1, "pic"))
        .filter_map(|node| {
            super::transform_info(&nodes, node).map(|info| (info.id, node.outer.clone()))
        })
        .collect())
}
