//! Writing to a story's XML: a field's new result, its lock, its code, or
//! the field taken out with its result kept (unlink).

use super::scan::{Marker, ScannedField};
use crate::docx_xml::{XmlAttribute, XmlElement, XmlNode, XmlNodeId};

/// What replaces a field's result.
#[derive(Clone, Debug)]
pub(crate) enum NewResult {
    /// Text; `\t` becomes a tab and `\n` a line break.
    Text(String),
    /// Paragraphs: the first continues the field's paragraph, each further
    /// one is a new paragraph like it.
    Paragraphs(Vec<String>),
    /// Text inside a bookmark (what ASK and SET leave for REF to read).
    Bookmarked { name: String, text: String },
    /// Prepared runs (a picture's).
    Runs(Vec<XmlElement>),
    /// A picture file's bytes and name, embedded by the caller.
    Picture { bytes: Vec<u8>, name: String },
}

fn new_element(name: &str, attributes: &[(&str, &str)], children: Vec<XmlNode>) -> XmlElement {
    XmlElement {
        id: XmlNodeId::new(0),
        name: name.to_string(),
        attributes: attributes
            .iter()
            .map(|(name, value)| XmlAttribute {
                name: name.to_string(),
                value: value.to_string(),
            })
            .collect(),
        children,
    }
}

/// Deep copy with fresh (placeholder) ids, so ids stay unique.
pub(crate) fn fresh_copy(element: &XmlElement) -> XmlElement {
    XmlElement {
        id: XmlNodeId::new(0),
        name: element.name.clone(),
        attributes: element.attributes.clone(),
        children: element
            .children
            .iter()
            .map(|child| match child {
                XmlNode::Element(child) => XmlNode::Element(fresh_copy(child)),
                other => other.clone(),
            })
            .collect(),
    }
}

/// Runs for `text` with properties `rpr`.
pub(crate) fn text_runs(text: &str, rpr: Option<&XmlElement>) -> Vec<XmlElement> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut children = Vec::new();
    if let Some(rpr) = rpr {
        children.push(XmlNode::Element(fresh_copy(rpr)));
    }
    let mut segment = String::new();
    let flush = |segment: &mut String, children: &mut Vec<XmlNode>| {
        if !segment.is_empty() {
            children.push(XmlNode::Element(new_element(
                "w:t",
                &[("xml:space", "preserve")],
                vec![XmlNode::Text(std::mem::take(segment))],
            )));
        }
    };
    for character in text.chars() {
        match character {
            '\t' => {
                flush(&mut segment, &mut children);
                children.push(XmlNode::Element(new_element("w:tab", &[], Vec::new())));
            }
            '\n' => {
                flush(&mut segment, &mut children);
                children.push(XmlNode::Element(new_element("w:br", &[], Vec::new())));
            }
            other => segment.push(other),
        }
    }
    flush(&mut segment, &mut children);
    vec![new_element("w:r", &[], children)]
}

/// The child path from `root` to the element `id`.
pub(crate) fn find_path(root: &XmlElement, id: XmlNodeId) -> Option<Vec<usize>> {
    fn walk(element: &XmlElement, id: XmlNodeId, path: &mut Vec<usize>) -> bool {
        if element.id == id {
            return true;
        }
        for (index, child) in element.children.iter().enumerate() {
            if let XmlNode::Element(child) = child {
                path.push(index);
                if walk(child, id, path) {
                    return true;
                }
                path.pop();
            }
        }
        false
    }
    let mut path = Vec::new();
    walk(root, id, &mut path).then_some(path)
}

pub(crate) fn at_path<'a>(root: &'a XmlElement, path: &[usize]) -> Option<&'a XmlElement> {
    let mut element = root;
    for &index in path {
        element = match element.children.get(index)? {
            XmlNode::Element(child) => child,
            _ => return None,
        };
    }
    Some(element)
}

pub(crate) fn at_path_mut<'a>(
    root: &'a mut XmlElement,
    path: &[usize],
) -> Option<&'a mut XmlElement> {
    let mut element = root;
    for &index in path {
        element = match element.children.get_mut(index)? {
            XmlNode::Element(child) => child,
            _ => return None,
        };
    }
    Some(element)
}

pub(crate) fn find_element<'a>(root: &'a XmlElement, id: XmlNodeId) -> Option<&'a XmlElement> {
    let path = find_path(root, id)?;
    at_path(root, &path)
}

pub(crate) fn find_element_mut<'a>(
    root: &'a mut XmlElement,
    id: XmlNodeId,
) -> Option<&'a mut XmlElement> {
    let path = find_path(root, id)?;
    at_path_mut(root, &path)
}

pub(crate) fn run_properties(run: &XmlElement) -> Option<&XmlElement> {
    run.children.iter().find_map(|child| match child {
        XmlNode::Element(child) if child.name == "w:rPr" => Some(child),
        _ => None,
    })
}

/// Nodes a result rewrite keeps where they are: anchors, not content.
fn is_anchor(node: &XmlNode) -> bool {
    matches!(node, XmlNode::Element(element) if matches!(
        element.name.as_str(),
        "w:bookmarkStart" | "w:bookmarkEnd" | "w:commentRangeStart" | "w:commentRangeEnd"
            | "w:permStart" | "w:permEnd"
    ))
}

fn is_marker_type(element: &XmlElement, kind: &str) -> bool {
    element.name == "w:fldChar"
        && element
            .attributes
            .iter()
            .any(|attribute| attribute.name == "w:fldCharType" && attribute.value == kind)
}

/// The path of a marker's run, re-found by the marker's type in case the
/// run's children moved.
fn marker_path(root: &XmlElement, marker: Marker) -> Option<Vec<usize>> {
    find_path(root, marker.run)
}

/// Removes everything between the separate and end markers of a complex
/// field (anchors stay), joining the end's paragraph to the separate's when
/// they differ. Returns the path of the separate run afterwards.
fn clear_complex_result(
    root: &mut XmlElement,
    separate: Marker,
    end: Marker,
) -> Option<Vec<usize>> {
    let start = marker_path(root, separate)?;
    let finish = marker_path(root, end)?;
    if start == finish {
        // Both markers in one run: drop the run children between them.
        let run = at_path_mut(root, &start)?;
        let separate_at = run.children.iter().position(
            |child| matches!(child, XmlNode::Element(e) if is_marker_type(e, "separate")),
        )?;
        let end_at = run
            .children
            .iter()
            .position(|child| matches!(child, XmlNode::Element(e) if is_marker_type(e, "end")))?;
        if end_at > separate_at + 1 {
            run.children.drain(separate_at + 1..end_at);
        }
        return Some(start);
    }
    let common = start
        .iter()
        .zip(&finish)
        .take_while(|(a, b)| a == b)
        .count();
    if common >= start.len() || common >= finish.len() {
        return None;
    }
    // Inside the end's branch, everything before the end run (deepest
    // first, so shallower indices stay valid).
    for depth in (common + 1..finish.len()).rev() {
        let parent = at_path_mut(root, &finish[..depth])?;
        let keep_from = finish[depth];
        let removed: Vec<XmlNode> = parent.children.drain(..keep_from).collect();
        let anchors: Vec<XmlNode> = removed.into_iter().filter(is_anchor).collect();
        let count = anchors.len();
        parent.children.splice(0..0, anchors);
        let _ = count;
    }
    // Inside the separate's branch, everything after the separate run.
    for depth in (common + 1..start.len()).rev() {
        let parent = at_path_mut(root, &start[..depth])?;
        let keep_to = start[depth] + 1;
        let removed: Vec<XmlNode> = parent.children.drain(keep_to..).collect();
        parent
            .children
            .extend(removed.into_iter().filter(is_anchor));
    }
    // Between the two branches.
    let container = at_path_mut(root, &start[..common])?;
    let (from, to) = (start[common] + 1, finish[common]);
    if to > from {
        let removed: Vec<XmlNode> = container.children.drain(from..to).collect();
        let anchors: Vec<XmlNode> = removed.into_iter().filter(|node| is_anchor(node)).collect();
        // Anchors between paragraphs belong in a paragraph; keep them only
        // when the container holds runs.
        if container.name == "w:p" || container.name == "w:hyperlink" {
            let count = anchors.len();
            container.children.splice(from..from, anchors);
            let _ = count;
        }
    }
    // Two paragraphs: the end's joins the separate's.
    let container = at_path_mut(root, &start[..common])?;
    let first = container.children.get(start[common]);
    let second_index = start[common] + 1;
    let both_paragraphs = matches!(first, Some(XmlNode::Element(e)) if e.name == "w:p")
        && matches!(container.children.get(second_index), Some(XmlNode::Element(e)) if e.name == "w:p");
    if both_paragraphs && container.name != "w:p" {
        let XmlNode::Element(second) = container.children.remove(second_index) else {
            unreachable!("checked above");
        };
        if let Some(XmlNode::Element(first)) = container.children.get_mut(start[common]) {
            // The joined paragraph keeps the first paragraph's properties
            // but the section break, if any, of the second.
            let second_section = second.children.iter().find_map(|child| match child {
                XmlNode::Element(pr) if pr.name == "w:pPr" => pr
                    .children
                    .iter()
                    .find(|node| matches!(node, XmlNode::Element(e) if e.name == "w:sectPr"))
                    .cloned(),
                _ => None,
            });
            first.children.extend(
                second
                    .children
                    .into_iter()
                    .filter(|child| !matches!(child, XmlNode::Element(e) if e.name == "w:pPr")),
            );
            if let Some(section) = second_section {
                if let Some(XmlNode::Element(pr)) = first
                    .children
                    .iter_mut()
                    .find(|node| matches!(node, XmlNode::Element(e) if e.name == "w:pPr"))
                {
                    pr.children.retain(
                        |node| !matches!(node, XmlNode::Element(e) if e.name == "w:sectPr"),
                    );
                    pr.children.push(section);
                }
            }
        }
    }
    marker_path(root, separate)
}

/// The formatting a new result takes: the first result run's, else the
/// field name's (`\* CHARFORMAT` asks for that one), else the begin run's.
pub(crate) fn result_properties(
    root: &XmlElement,
    field: &ScannedField,
    char_format: bool,
) -> Option<XmlElement> {
    let from = |id: XmlNodeId| find_element(root, id).and_then(run_properties).cloned();
    if char_format {
        if let Some(rpr) = field.instruction_runs.first().and_then(|&id| from(id)) {
            return Some(rpr);
        }
    }
    field
        .result_runs
        .first()
        .and_then(|&id| from(id))
        .or_else(|| field.instruction_runs.first().and_then(|&id| from(id)))
        .or_else(|| field.begin.and_then(|marker| from(marker.run)))
        .or_else(|| {
            field.simple.and_then(|id| {
                find_element(root, id).and_then(|simple| {
                    simple.children.iter().find_map(|child| match child {
                        XmlNode::Element(run) if run.name == "w:r" => run_properties(run).cloned(),
                        _ => None,
                    })
                })
            })
        })
}

/// `\* MERGEFORMAT`: the new text laid over the old result runs, each run
/// keeping its formatting for as many characters as it held.
pub(crate) fn merge_format_runs(
    root: &XmlElement,
    field: &ScannedField,
    text: &str,
) -> Vec<XmlElement> {
    let old: Vec<(usize, Option<XmlElement>)> = field
        .result_runs
        .iter()
        .filter_map(|&id| find_element(root, id))
        .map(|run| {
            let length = run
                .children
                .iter()
                .map(|child| match child {
                    XmlNode::Element(e) if e.name == "w:t" => e
                        .children
                        .iter()
                        .map(|node| match node {
                            XmlNode::Text(text) => text.chars().count(),
                            _ => 0,
                        })
                        .sum(),
                    XmlNode::Element(e) if matches!(e.name.as_str(), "w:tab" | "w:br" | "w:cr") => {
                        1
                    }
                    _ => 0,
                })
                .sum::<usize>();
            (length, run_properties(run).cloned())
        })
        .collect();
    let chars: Vec<char> = text.chars().collect();
    let mut runs = Vec::new();
    let mut at = 0;
    for (index, (length, rpr)) in old.iter().enumerate() {
        let take = if index + 1 == old.len() {
            chars.len() - at
        } else {
            (*length).min(chars.len() - at)
        };
        let piece: String = chars[at..at + take].iter().collect();
        at += take;
        runs.extend(text_runs(&piece, rpr.as_ref()));
        if at >= chars.len() {
            break;
        }
    }
    runs
}

fn next_bookmark_id(root: &XmlElement) -> u32 {
    fn walk(element: &XmlElement, max: &mut u32) {
        if element.name == "w:bookmarkStart" {
            if let Some(id) = element
                .attributes
                .iter()
                .find(|attribute| attribute.name == "w:id")
                .and_then(|attribute| attribute.value.parse::<u32>().ok())
            {
                *max = (*max).max(id + 1);
            }
        }
        for child in &element.children {
            if let XmlNode::Element(child) = child {
                walk(child, max);
            }
        }
    }
    let mut max = 0;
    walk(root, &mut max);
    max
}

/// Clears `dirty` from a field's begin marker or simple field: an updated
/// field is current.
fn clear_dirty(root: &mut XmlElement, field: &ScannedField) {
    let target = field
        .simple
        .or(field.begin.map(|marker| marker.run))
        .and_then(|id| find_element_mut(root, id));
    let Some(target) = target else { return };
    if target.name == "w:fldSimple" {
        target
            .attributes
            .retain(|attribute| attribute.name != "w:dirty");
        return;
    }
    for child in &mut target.children {
        if let XmlNode::Element(child) = child {
            if is_marker_type(child, "begin") {
                child
                    .attributes
                    .retain(|attribute| attribute.name != "w:dirty");
            }
        }
    }
}

/// Replaces a field's result. `runs` are the result runs for a text result
/// (built by the caller to carry the right formatting).
pub(crate) fn write_result(
    root: &mut XmlElement,
    field: &ScannedField,
    result: NewResult,
    rpr: Option<&XmlElement>,
    prepared_runs: Option<Vec<XmlElement>>,
) -> Result<(), String> {
    clear_dirty(root, field);
    let mut nodes: Vec<XmlElement> = match &result {
        NewResult::Text(text) => prepared_runs.unwrap_or_else(|| text_runs(text, rpr)),
        NewResult::Paragraphs(paragraphs) => text_runs(
            paragraphs.first().map(String::as_str).unwrap_or_default(),
            rpr,
        ),
        NewResult::Bookmarked { name, text } => {
            let id = next_bookmark_id(root).to_string();
            let mut nodes = vec![new_element(
                "w:bookmarkStart",
                &[("w:id", &id), ("w:name", name)],
                Vec::new(),
            )];
            nodes.extend(text_runs(text, rpr));
            nodes.push(new_element("w:bookmarkEnd", &[("w:id", &id)], Vec::new()));
            nodes
        }
        NewResult::Runs(runs) => runs.clone(),
        NewResult::Picture { .. } => return Err("a picture result needs its package".to_string()),
    };

    if let Some(simple) = field.simple {
        let element = find_element_mut(root, simple).ok_or("simple field not found")?;
        element.children.retain(|child| {
            !matches!(child, XmlNode::Element(e) if matches!(e.name.as_str(), "w:r" | "w:hyperlink" | "w:smartTag" | "w:ins" | "w:fldSimple"))
        });
        element
            .children
            .extend(nodes.into_iter().map(XmlNode::Element));
        return Ok(());
    }

    let (Some(begin), Some(end)) = (field.begin, field.end) else {
        return Err("field has no end".to_string());
    };
    let separate_path = match field.separate {
        Some(separate) => {
            clear_complex_result(root, separate, end).ok_or("field result could not be cleared")?
        }
        None => {
            // Instruction only: a separate marker goes before the end.
            let end_path = marker_path(root, end).ok_or("field end not found")?;
            let (parent_path, index) = end_path.split_at(end_path.len() - 1);
            let parent = at_path_mut(root, parent_path).ok_or("field end parent not found")?;
            let end_run = match &parent.children[index[0]] {
                XmlNode::Element(run) => run.clone(),
                _ => return Err("field end is not a run".to_string()),
            };
            let mut separate_run = new_element("w:r", &[], Vec::new());
            if let Some(rpr) = run_properties(&end_run) {
                separate_run
                    .children
                    .push(XmlNode::Element(fresh_copy(rpr)));
            }
            separate_run.children.push(XmlNode::Element(new_element(
                "w:fldChar",
                &[("w:fldCharType", "separate")],
                Vec::new(),
            )));
            parent
                .children
                .insert(index[0], XmlNode::Element(separate_run));
            let mut path = parent_path.to_vec();
            path.push(index[0]);
            path
        }
    };
    let _ = begin;
    let (parent_path, index) = separate_path.split_at(separate_path.len() - 1);
    let separate_index = index[0];
    let same_run_end = at_path(root, &separate_path).is_some_and(|run| {
        run.children
            .iter()
            .any(|child| matches!(child, XmlNode::Element(e) if is_marker_type(e, "end")))
    });
    if same_run_end {
        // Split the run: separate stays, the end moves to a run after the
        // result.
        let run = at_path_mut(root, &separate_path).ok_or("separate run lost")?;
        let end_at = run
            .children
            .iter()
            .position(|child| matches!(child, XmlNode::Element(e) if is_marker_type(e, "end")))
            .ok_or("end marker lost")?;
        let tail: Vec<XmlNode> = run.children.drain(end_at..).collect();
        let mut end_run = new_element("w:r", &[], Vec::new());
        if let Some(rpr) = run_properties(run) {
            end_run.children.push(XmlNode::Element(fresh_copy(rpr)));
        }
        end_run.children.extend(tail);
        nodes.push(end_run);
    }

    let paragraphs = match result {
        NewResult::Paragraphs(paragraphs) if paragraphs.len() > 1 => paragraphs,
        _ => {
            let parent = at_path_mut(root, parent_path).ok_or("separate parent lost")?;
            let at = separate_index + 1;
            parent
                .children
                .splice(at..at, nodes.into_iter().map(XmlNode::Element));
            return Ok(());
        }
    };
    // Several paragraphs need the separate run directly in its paragraph.
    let parent = at_path(root, parent_path).ok_or("separate parent lost")?;
    if parent.name != "w:p" || parent_path.is_empty() {
        let joined = paragraphs.join("\n");
        let parent = at_path_mut(root, parent_path).ok_or("separate parent lost")?;
        let at = separate_index + 1;
        parent.children.splice(
            at..at,
            text_runs(&joined, rpr).into_iter().map(XmlNode::Element),
        );
        return Ok(());
    }
    let (container_path, paragraph_index) = parent_path.split_at(parent_path.len() - 1);
    let paragraph_index = paragraph_index[0];
    let container = at_path_mut(root, container_path).ok_or("paragraph container lost")?;
    let XmlNode::Element(paragraph) = &mut container.children[paragraph_index] else {
        return Err("paragraph lost".to_string());
    };
    let at = separate_index + 1;
    let tail: Vec<XmlNode> = paragraph.children.drain(at..).collect();
    paragraph
        .children
        .extend(nodes.into_iter().map(XmlNode::Element));
    let properties = paragraph.children.iter().find_map(|child| match child {
        XmlNode::Element(e) if e.name == "w:pPr" => Some(e.clone()),
        _ => None,
    });
    // The section break stays with the last paragraph.
    let mut section = None;
    if let Some(XmlNode::Element(pr)) = paragraph
        .children
        .iter_mut()
        .find(|node| matches!(node, XmlNode::Element(e) if e.name == "w:pPr"))
    {
        if let Some(position) = pr
            .children
            .iter()
            .position(|node| matches!(node, XmlNode::Element(e) if e.name == "w:sectPr"))
        {
            section = Some(pr.children.remove(position));
        }
    }
    let count = paragraphs.len();
    let mut new_paragraphs = Vec::new();
    for (index, text) in paragraphs.iter().enumerate().skip(1) {
        let mut children = Vec::new();
        if let Some(pr) = &properties {
            let mut pr = fresh_copy(pr);
            pr.children
                .retain(|node| !matches!(node, XmlNode::Element(e) if e.name == "w:sectPr"));
            if index + 1 == count {
                if let Some(section) = section.clone() {
                    pr.children.push(section);
                }
            }
            children.push(XmlNode::Element(pr));
        }
        children.extend(text_runs(text, rpr).into_iter().map(XmlNode::Element));
        if index + 1 == count {
            children.extend(tail.clone());
        }
        new_paragraphs.push(XmlNode::Element(new_element("w:p", &[], children)));
    }
    container
        .children
        .splice(paragraph_index + 1..paragraph_index + 1, new_paragraphs);
    Ok(())
}

/// Locks or unlocks a field (`w:fldLock`).
pub(crate) fn set_locked(root: &mut XmlElement, field: &ScannedField, locked: bool) -> bool {
    let apply = |attributes: &mut Vec<XmlAttribute>| {
        attributes.retain(|attribute| attribute.name != "w:fldLock");
        if locked {
            attributes.push(XmlAttribute {
                name: "w:fldLock".to_string(),
                value: "1".to_string(),
            });
        }
    };
    if let Some(simple) = field.simple {
        if let Some(element) = find_element_mut(root, simple) {
            apply(&mut element.attributes);
            return true;
        }
        return false;
    }
    let Some(begin) = field.begin else {
        return false;
    };
    let Some(run) = find_element_mut(root, begin.run) else {
        return false;
    };
    for child in &mut run.children {
        if let XmlNode::Element(child) = child {
            if is_marker_type(child, "begin") {
                apply(&mut child.attributes);
                return true;
            }
        }
    }
    false
}

/// Whether a run has nothing left but its properties.
fn run_is_empty(run: &XmlElement) -> bool {
    run.children
        .iter()
        .all(|child| matches!(child, XmlNode::Element(e) if e.name == "w:rPr"))
}

/// Removes a field's code and markers, leaving its result as plain text.
/// `fields` are the story's fields: the operands nested in the code go
/// with it.
pub(crate) fn unlink(root: &mut XmlElement, field: &ScannedField) -> Result<(), String> {
    if let Some(simple) = field.simple {
        let path = find_path(root, simple).ok_or("simple field not found")?;
        let (parent_path, index) = path.split_at(path.len() - 1);
        let parent = at_path_mut(root, parent_path).ok_or("simple field parent lost")?;
        let XmlNode::Element(element) = parent.children.remove(index[0]) else {
            return Err("simple field is not an element".to_string());
        };
        parent.children.splice(index[0]..index[0], element.children);
        return Ok(());
    }
    let (Some(begin), Some(end)) = (field.begin, field.end) else {
        return Err("field has no end".to_string());
    };
    // Everything from the begin marker to the separate marker (or to the end
    // when there is no result) is code; the end marker goes too.
    let code_end = field.separate.unwrap_or(end);
    remove_between(root, begin, code_end)?;
    remove_marker(root, end)?;
    Ok(())
}

/// Removes the markers `from` and `to` and everything between them.
fn remove_between(root: &mut XmlElement, from: Marker, to: Marker) -> Result<(), String> {
    let start = find_path(root, from.run).ok_or("begin run not found")?;
    let finish = find_path(root, to.run).ok_or("separate run not found")?;
    if start == finish {
        let run = at_path_mut(root, &start).ok_or("run lost")?;
        let first = run
            .children
            .iter()
            .position(|child| matches!(child, XmlNode::Element(e) if is_marker_type(e, "begin")))
            .unwrap_or(0);
        let last = run
            .children
            .iter()
            .rposition(|child| {
                matches!(child, XmlNode::Element(e) if is_marker_type(e, "separate") || is_marker_type(e, "end"))
            })
            .unwrap_or(run.children.len() - 1);
        run.children.drain(first..=last);
        if run_is_empty(run) {
            let (parent_path, index) = start.split_at(start.len() - 1);
            at_path_mut(root, parent_path)
                .ok_or("run parent lost")?
                .children
                .remove(index[0]);
        }
        return Ok(());
    }
    let common = start
        .iter()
        .zip(&finish)
        .take_while(|(a, b)| a == b)
        .count();
    if common >= start.len() || common >= finish.len() {
        return Err("field markers are nested in one another".to_string());
    }
    // The finishing run: drop children up to and including the marker.
    {
        let run = at_path_mut(root, &finish).ok_or("run lost")?;
        let marker_at = run
            .children
            .iter()
            .position(|child| {
                matches!(child, XmlNode::Element(e) if is_marker_type(e, "separate") || is_marker_type(e, "end"))
            })
            .unwrap_or(run.children.len().saturating_sub(1));
        let rpr_kept: Vec<XmlNode> = run
            .children
            .drain(..=marker_at)
            .filter(|node| matches!(node, XmlNode::Element(e) if e.name == "w:rPr"))
            .collect();
        run.children.splice(0..0, rpr_kept);
    }
    for depth in (common + 1..finish.len()).rev() {
        let parent = at_path_mut(root, &finish[..depth]).ok_or("path lost")?;
        let keep_from = finish[depth];
        let removed: Vec<XmlNode> = parent.children.drain(..keep_from).collect();
        let anchors: Vec<XmlNode> = removed.into_iter().filter(is_anchor).collect();
        parent.children.splice(0..0, anchors);
    }
    {
        let run = at_path_mut(root, &start).ok_or("run lost")?;
        let marker_at = run
            .children
            .iter()
            .position(|child| matches!(child, XmlNode::Element(e) if is_marker_type(e, "begin")))
            .unwrap_or(0);
        run.children.truncate(marker_at);
    }
    for depth in (common + 1..start.len()).rev() {
        let parent = at_path_mut(root, &start[..depth]).ok_or("path lost")?;
        let keep_to = start[depth] + 1;
        let removed: Vec<XmlNode> = parent.children.drain(keep_to..).collect();
        parent
            .children
            .extend(removed.into_iter().filter(is_anchor));
    }
    let container = at_path_mut(root, &start[..common]).ok_or("container lost")?;
    let (from_index, to_index) = (start[common] + 1, finish[common]);
    if to_index > from_index {
        let removed: Vec<XmlNode> = container.children.drain(from_index..to_index).collect();
        let anchors: Vec<XmlNode> = removed.into_iter().filter(is_anchor).collect();
        container.children.splice(from_index..from_index, anchors);
    }
    // Runs emptied by the removal go.
    for id in [from.run, to.run] {
        if let Some(path) = find_path(root, id) {
            if at_path(root, &path).is_some_and(run_is_empty) {
                let (parent_path, index) = path.split_at(path.len() - 1);
                if let Some(parent) = at_path_mut(root, parent_path) {
                    parent.children.remove(index[0]);
                }
            }
        }
    }
    Ok(())
}

fn remove_marker(root: &mut XmlElement, marker: Marker) -> Result<(), String> {
    let path = find_path(root, marker.run).ok_or("end run not found")?;
    let run = at_path_mut(root, &path).ok_or("end run lost")?;
    if let Some(at) = run
        .children
        .iter()
        .position(|child| matches!(child, XmlNode::Element(e) if is_marker_type(e, "end")))
    {
        run.children.remove(at);
    }
    if run_is_empty(run) {
        let (parent_path, index) = path.split_at(path.len() - 1);
        at_path_mut(root, parent_path)
            .ok_or("end run parent lost")?
            .children
            .remove(index[0]);
    }
    Ok(())
}

/// Replaces a field's code with `code` (which may hold nested fields in
/// braces). A simple field becomes a complex one when the code nests.
pub(crate) fn replace_code(
    root: &mut XmlElement,
    field: &ScannedField,
    code: &str,
) -> Result<(), String> {
    let rpr = field
        .instruction_runs
        .first()
        .and_then(|&id| find_element(root, id))
        .and_then(run_properties)
        .cloned()
        .or_else(|| result_properties(root, field, false));
    if let Some(simple) = field.simple {
        if !code.contains('{') {
            let element = find_element_mut(root, simple).ok_or("simple field not found")?;
            element
                .attributes
                .retain(|attribute| attribute.name != "w:instr");
            element.attributes.push(XmlAttribute {
                name: "w:instr".to_string(),
                value: format!(" {} ", code.trim()),
            });
            return Ok(());
        }
        // Rebuild as a complex field around the current result.
        let path = find_path(root, simple).ok_or("simple field not found")?;
        let (parent_path, index) = path.split_at(path.len() - 1);
        let parent = at_path_mut(root, parent_path).ok_or("simple field parent lost")?;
        let XmlNode::Element(element) = parent.children.remove(index[0]) else {
            return Err("simple field is not an element".to_string());
        };
        let mut runs = code_runs(code, rpr.as_ref(), true);
        runs.push(marker_run("separate", rpr.as_ref()));
        let mut nodes: Vec<XmlNode> = runs.into_iter().map(XmlNode::Element).collect();
        nodes.extend(element.children);
        nodes.push(XmlNode::Element(marker_run("end", rpr.as_ref())));
        parent.children.splice(index[0]..index[0], nodes);
        return Ok(());
    }
    let (Some(begin), Some(end)) = (field.begin, field.end) else {
        return Err("field has no end".to_string());
    };
    let code_end = field.separate.unwrap_or(end);
    // Clear the old code: keep the begin and separate (or end) markers.
    let begin_path = find_path(root, begin.run).ok_or("begin run not found")?;
    let separate_path = find_path(root, code_end.run).ok_or("separate run not found")?;
    if begin_path == separate_path {
        let run = at_path_mut(root, &begin_path).ok_or("run lost")?;
        let begin_at = run
            .children
            .iter()
            .position(|child| matches!(child, XmlNode::Element(e) if is_marker_type(e, "begin")))
            .ok_or("begin marker lost")?;
        let tail: Vec<XmlNode> = run.children.drain(begin_at + 1..).collect();
        let keep: Vec<XmlNode> = tail
            .into_iter()
            .skip_while(|node| {
                !matches!(node, XmlNode::Element(e) if is_marker_type(e, "separate") || is_marker_type(e, "end"))
            })
            .collect();
        let mut split = new_element("w:r", &[], Vec::new());
        if let Some(rpr) = run_properties(run) {
            split.children.push(XmlNode::Element(fresh_copy(rpr)));
        }
        split.children.extend(keep);
        let (parent_path, index) = begin_path.split_at(begin_path.len() - 1);
        let parent = at_path_mut(root, parent_path).ok_or("run parent lost")?;
        let mut inserted: Vec<XmlNode> = code_runs(code, rpr.as_ref(), false)
            .into_iter()
            .map(XmlNode::Element)
            .collect();
        inserted.push(XmlNode::Element(split));
        parent.children.splice(index[0] + 1..index[0] + 1, inserted);
        return Ok(());
    }
    // Remove what lies between the begin run and the separate run.
    {
        let run = at_path_mut(root, &begin_path).ok_or("run lost")?;
        if let Some(at) = run
            .children
            .iter()
            .position(|child| matches!(child, XmlNode::Element(e) if is_marker_type(e, "begin")))
        {
            run.children.truncate(at + 1);
        }
    }
    {
        let run = at_path_mut(root, &separate_path).ok_or("run lost")?;
        if let Some(at) = run.children.iter().position(|child| {
            matches!(child, XmlNode::Element(e) if is_marker_type(e, "separate") || is_marker_type(e, "end"))
        }) {
            let rpr: Vec<XmlNode> = run
                .children
                .drain(..at)
                .filter(|node| matches!(node, XmlNode::Element(e) if e.name == "w:rPr"))
                .collect();
            run.children.splice(0..0, rpr);
        }
    }
    let common = begin_path
        .iter()
        .zip(&separate_path)
        .take_while(|(a, b)| a == b)
        .count();
    if common + 1 != begin_path.len() || common + 1 != separate_path.len() {
        return Err("the field code spans several paragraphs".to_string());
    }
    let parent = at_path_mut(root, &begin_path[..common]).ok_or("container lost")?;
    let (from, to) = (begin_path[common] + 1, separate_path[common]);
    let removed: Vec<XmlNode> = parent.children.drain(from..to).collect();
    let mut inserted: Vec<XmlNode> = removed.into_iter().filter(is_anchor).collect();
    inserted.extend(
        code_runs(code, rpr.as_ref(), false)
            .into_iter()
            .map(XmlNode::Element),
    );
    parent.children.splice(from..from, inserted);
    Ok(())
}

fn marker_run(kind: &str, rpr: Option<&XmlElement>) -> XmlElement {
    let mut children = Vec::new();
    if let Some(rpr) = rpr {
        children.push(XmlNode::Element(fresh_copy(rpr)));
    }
    children.push(XmlNode::Element(new_element(
        "w:fldChar",
        &[("w:fldCharType", kind)],
        Vec::new(),
    )));
    new_element("w:r", &[], children)
}

fn instruction_run(text: &str, rpr: Option<&XmlElement>) -> XmlElement {
    let mut children = Vec::new();
    if let Some(rpr) = rpr {
        children.push(XmlNode::Element(fresh_copy(rpr)));
    }
    children.push(XmlNode::Element(new_element(
        "w:instrText",
        &[("xml:space", "preserve")],
        vec![XmlNode::Text(text.to_string())],
    )));
    new_element("w:r", &[], children)
}

/// The runs of a field code: instruction text, with each `{ … }` a nested
/// field (begin, its code, end — its result comes with the next update).
/// With `with_begin` the outer begin marker leads.
pub(crate) fn code_runs(code: &str, rpr: Option<&XmlElement>, with_begin: bool) -> Vec<XmlElement> {
    let mut runs = Vec::new();
    if with_begin {
        runs.push(marker_run("begin", rpr));
    }
    let mut text = String::new();
    let flush = |text: &mut String, runs: &mut Vec<XmlElement>| {
        if !text.is_empty() {
            runs.push(instruction_run(&std::mem::take(text), rpr));
        }
    };
    let mut in_quotes = false;
    for character in code.chars() {
        match character {
            '"' => {
                in_quotes = !in_quotes;
                text.push(character);
            }
            '{' if !in_quotes => {
                flush(&mut text, &mut runs);
                runs.push(marker_run("begin", rpr));
            }
            '}' if !in_quotes => {
                flush(&mut text, &mut runs);
                runs.push(marker_run("separate", rpr));
                runs.push(marker_run("end", rpr));
            }
            _ => text.push(character),
        }
    }
    if !text.starts_with(' ') && runs.is_empty() {
        text.insert(0, ' ');
    }
    flush(&mut text, &mut runs);
    runs
}

/// The runs of a whole new field: begin, code, separate, result, end.
pub(crate) fn field_runs(code: &str, result: &str, rpr: Option<&XmlElement>) -> Vec<XmlElement> {
    let mut runs = code_runs(&format!(" {} ", code.trim()), rpr, true);
    runs.push(marker_run("separate", rpr));
    runs.extend(text_runs(result, rpr));
    runs.push(marker_run("end", rpr));
    runs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docx_xml::DocxXmlDocument;
    use crate::fields::scan::scan_document;

    const W: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"";

    fn doc(body: &str) -> DocxXmlDocument {
        DocxXmlDocument::parse(&format!(
            "<w:document {W}><w:body>{body}</w:body></w:document>"
        ))
        .unwrap()
    }

    fn body_xml(doc: &DocxXmlDocument) -> String {
        let xml = doc.to_string().unwrap();
        let start = xml.find("<w:body>").unwrap() + "<w:body>".len();
        let end = xml.find("</w:body>").unwrap();
        xml[start..end].to_string()
    }

    fn root_mut(doc: &mut DocxXmlDocument) -> &mut XmlElement {
        let id = doc.root().id;
        doc.element_mut(id).unwrap()
    }

    const COMPLEX: &str = concat!(
        "<w:p><w:r><w:t xml:space=\"preserve\">A </w:t></w:r>",
        "<w:r><w:fldChar w:fldCharType=\"begin\" w:dirty=\"true\"/></w:r>",
        "<w:r><w:instrText xml:space=\"preserve\"> NUMPAGES </w:instrText></w:r>",
        "<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r>",
        "<w:r><w:rPr><w:b/></w:rPr><w:t>1</w:t></w:r>",
        "<w:r><w:t>0</w:t></w:r>",
        "<w:r><w:fldChar w:fldCharType=\"end\"/></w:r>",
        "<w:r><w:t xml:space=\"preserve\"> B</w:t></w:r></w:p>"
    );

    #[test]
    fn writes_a_result_with_the_first_result_runs_formatting() {
        let mut doc = doc(COMPLEX);
        let story = scan_document(doc.root());
        let field = story.fields[0].clone();
        let root = root_mut(&mut doc);
        let rpr = result_properties(root, &field, false);
        write_result(
            root,
            &field,
            NewResult::Text("7".into()),
            rpr.as_ref(),
            None,
        )
        .unwrap();
        let xml = body_xml(&doc);
        assert!(xml.contains("<w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:rPr><w:b/></w:rPr><w:t xml:space=\"preserve\">7</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/>"), "{xml}");
        assert!(!xml.contains("w:dirty"));
        let story = scan_document(doc.root());
        assert_eq!(story.fields[0].result_text, "7");
        assert_eq!(story.paragraphs[0].text, "A 7 B");
    }

    #[test]
    fn writes_a_result_across_paragraphs_and_into_new_ones() {
        let mut doc = doc(concat!(
            "<w:p><w:r><w:fldChar w:fldCharType=\"begin\"/></w:r>",
            "<w:r><w:instrText> INCLUDETEXT x </w:instrText></w:r>",
            "<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>one</w:t></w:r></w:p>",
            "<w:p><w:r><w:t>two</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r><w:r><w:t>!</w:t></w:r></w:p>"
        ));
        let field = scan_document(doc.root()).fields[0].clone();
        write_result(
            root_mut(&mut doc),
            &field,
            NewResult::Text("joined".into()),
            None,
            None,
        )
        .unwrap();
        let story = scan_document(doc.root());
        assert_eq!(story.paragraphs.len(), 1);
        assert_eq!(story.paragraphs[0].text, "joined!");

        let field = story.fields[0].clone();
        write_result(
            root_mut(&mut doc),
            &field,
            NewResult::Paragraphs(vec!["a".into(), "b".into(), "c".into()]),
            None,
            None,
        )
        .unwrap();
        let story = scan_document(doc.root());
        let texts: Vec<&str> = story.paragraphs.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts, vec!["a", "b", "c!"]);
        assert_eq!(story.fields[0].result_text, "abc");
    }

    #[test]
    fn locks_unlinks_and_replaces_codes() {
        let mut doc = doc(COMPLEX);
        let field = scan_document(doc.root()).fields[0].clone();
        assert!(set_locked(root_mut(&mut doc), &field, true));
        assert!(scan_document(doc.root()).fields[0].locked);
        assert!(set_locked(root_mut(&mut doc), &field, false));
        assert!(!scan_document(doc.root()).fields[0].locked);

        replace_code(root_mut(&mut doc), &field, "SECTIONPAGES \\* roman").unwrap();
        let story = scan_document(doc.root());
        assert_eq!(
            story.fields[0].instruction_with(|_| String::new()).trim(),
            "SECTIONPAGES \\* roman"
        );
        assert_eq!(story.fields[0].result_text, "10");

        let field = story.fields[0].clone();
        unlink(root_mut(&mut doc), &field).unwrap();
        let story = scan_document(doc.root());
        assert!(story.fields.is_empty());
        assert_eq!(story.paragraphs[0].text, "A 10 B");
        let xml = body_xml(&doc);
        assert!(
            !xml.contains("instrText") && !xml.contains("fldChar"),
            "{xml}"
        );
    }

    #[test]
    fn simple_fields_unlink_and_take_nested_codes() {
        let mut nested = doc(
            "<w:p><w:fldSimple w:instr=\" AUTHOR \"><w:r><w:t>Ann</w:t></w:r></w:fldSimple></w:p>",
        );
        let field = scan_document(nested.root()).fields[0].clone();
        replace_code(root_mut(&mut nested), &field, "IF { PAGE } = 1 \"a\" \"b\"").unwrap();
        let story = scan_document(nested.root());
        assert_eq!(story.fields.len(), 2);
        assert_eq!(
            story.fields[0].display_code(&story.fields).trim(),
            "IF { PAGE } = 1 \"a\" \"b\""
        );
        assert_eq!(story.fields[0].result_text, "Ann");

        let mut plain = doc(
            "<w:p><w:fldSimple w:instr=\" AUTHOR \"><w:r><w:t>Ann</w:t></w:r></w:fldSimple></w:p>",
        );
        let field = scan_document(plain.root()).fields[0].clone();
        unlink(root_mut(&mut plain), &field).unwrap();
        assert_eq!(body_xml(&plain), "<w:p><w:r><w:t>Ann</w:t></w:r></w:p>");
    }

    #[test]
    fn builds_whole_fields_from_codes_with_nesting() {
        let runs = field_runs("IF { DATE \\@ \"M\" } = 1 \"Jan\" \"\"", "x", None);
        let names: Vec<String> = runs
            .iter()
            .map(|run| match &run.children[0] {
                XmlNode::Element(e) => e.name.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(
            names,
            [
                "w:fldChar",
                "w:instrText",
                "w:fldChar",
                "w:instrText",
                "w:fldChar",
                "w:fldChar",
                "w:instrText",
                "w:fldChar",
                "w:t",
                "w:fldChar"
            ]
        );
    }
}
