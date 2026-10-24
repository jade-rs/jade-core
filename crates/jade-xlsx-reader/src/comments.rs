//! Legacy worksheet comments (called "notes" by current Excel).
use crate::{model::CellComment, Result};
/// Read only owned comments/authors. Rich text uses the same namespace/MCE,
/// entity, CDATA and phonetic-value policy as ordinary shared strings.
pub fn parse_comments(xml: &str) -> Result<Vec<CellComment>> {
    let projected = crate::cell_content::comments(xml)?;
    let source = crate::object_protection::comments_source(&projected)?;
    Ok(source
        .comments
        .into_iter()
        .filter_map(|c| {
            c.at.map(|reference| CellComment {
                reference,
                author: c.author_id.and_then(|i| source.authors.get(i).cloned()),
                text: c.value,
            })
        })
        .collect())
}
