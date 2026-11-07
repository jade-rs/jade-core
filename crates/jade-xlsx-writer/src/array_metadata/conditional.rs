//! Plan additions against every stored metadata layout, then splice once.
use super::{
    append, apply, escape, nodes, valid_count, valid_namespace, worksheet_metadata_links, Node,
    OpcPackage, Result, Splice,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};

const MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
type Choices = BTreeMap<usize, Option<usize>>;

struct Projection {
    nodes: Vec<usize>,
    depths: Vec<usize>,
    // Empty selected branches are insertion slots at a logical child boundary.
    slots: Vec<(usize, usize, usize)>,
}
struct View<'a> {
    tree: &'a [Node],
    choices: &'a Choices,
    visited: RefCell<BTreeSet<usize>>,
}
impl View<'_> {
    fn children(&self, parent: usize) -> Projection {
        let mut pending: Vec<_> = self.tree[parent]
            .children
            .iter()
            .rev()
            .map(|&index| (index, 0))
            .collect();
        let mut result = Projection {
            nodes: Vec::new(),
            depths: Vec::new(),
            slots: Vec::new(),
        };
        while let Some((index, depth)) = pending.pop() {
            let node = &self.tree[index];
            if node.namespace != MC || node.name != "AlternateContent" {
                result.nodes.push(index);
                result.depths.push(depth);
                continue;
            }
            self.visited.borrow_mut().insert(index);
            let selected = self.choices.get(&index).copied().unwrap_or_else(|| {
                node.children
                    .iter()
                    .copied()
                    .find(|&i| self.tree[i].namespace == MC && self.tree[i].name == "Choice")
                    .or_else(|| {
                        node.children.iter().copied().find(|&i| {
                            self.tree[i].namespace == MC && self.tree[i].name == "Fallback"
                        })
                    })
            });
            if let Some(branch) = selected {
                if self.tree[branch].children.is_empty() {
                    result.slots.push((result.nodes.len(), branch, depth + 1));
                } else {
                    pending.extend(
                        self.tree[branch]
                            .children
                            .iter()
                            .rev()
                            .map(|&index| (index, depth + 1)),
                    );
                }
            }
        }
        result
    }
    fn owned(&self, parent: usize, name: &str) -> Vec<usize> {
        self.children(parent)
            .nodes
            .into_iter()
            .filter(|&i| {
                self.tree[i].namespace == self.tree[parent].namespace && self.tree[i].name == name
            })
            .collect()
    }
}

struct Layout {
    types: Option<usize>,
    cells: Option<usize>,
    future: Option<usize>,
    entries: Vec<usize>,
    blocks: Vec<usize>,
    records: Vec<usize>,
    links: Vec<usize>,
    root: Projection,
}
#[derive(Eq, PartialEq, Ord, PartialOrd)]
enum Anchor {
    Position(usize),
    EmptyBranch(usize),
}
fn layouts(tree: &[Node], root: usize) -> Option<Vec<Layout>> {
    let mut pending = vec![Choices::new()];
    let mut result = Vec::new();
    let mut references = 0usize;
    let reference_budget = tree.len().saturating_mul(8).max(65536);
    for _ in 0..1024 {
        let Some(choices) = pending.pop() else {
            return Some(result);
        };
        let view = View {
            tree,
            choices: &choices,
            visited: RefCell::default(),
        };
        let types = view.owned(root, "metadataTypes");
        let cells = view.owned(root, "cellMetadata");
        let futures: Vec<_> = view
            .owned(root, "futureMetadata")
            .into_iter()
            .filter(|&i| tree[i].attr("name") == Some("XLDAPR"))
            .collect();
        if types.len() > 1 || cells.len() > 1 || futures.len() > 1 {
            return None;
        }
        let entries = types
            .first()
            .map(|&i| view.owned(i, "metadataType"))
            .unwrap_or_default();
        let blocks = cells
            .first()
            .map(|&i| view.owned(i, "bk"))
            .unwrap_or_default();
        let records = futures
            .first()
            .map(|&i| view.owned(i, "bk"))
            .unwrap_or_default();
        let mut links = Vec::new();
        for kind in ["cellMetadata", "valueMetadata"] {
            for container in view.owned(root, kind) {
                for block in view.owned(container, "bk") {
                    links.extend(view.owned(block, "rc"));
                }
            }
        }
        let projection = view.children(root);
        let next = view
            .visited
            .borrow()
            .iter()
            .copied()
            .find(|i| !choices.contains_key(i));
        if let Some(next) = next {
            let branches: Vec<_> = tree[next]
                .children
                .iter()
                .copied()
                .filter(|&i| {
                    tree[i].namespace == MC
                        && matches!(tree[i].name.as_str(), "Choice" | "Fallback")
                })
                .collect();
            if !branches.iter().any(|&i| tree[i].name == "Fallback") {
                let mut empty = choices.clone();
                empty.insert(next, None);
                pending.push(empty);
            }
            for branch in branches {
                let mut selected = choices.clone();
                selected.insert(next, Some(branch));
                pending.push(selected);
            }
        } else {
            let used = entries
                .len()
                .saturating_add(blocks.len())
                .saturating_add(records.len())
                .saturating_add(links.len())
                .saturating_add(projection.nodes.len())
                .saturating_add(projection.depths.len())
                .saturating_add(projection.slots.len().saturating_mul(3));
            references = references
                .checked_add(used)
                .filter(|&n| n <= reference_budget)?;
            result.push(Layout {
                types: types.first().copied(),
                cells: cells.first().copied(),
                future: futures.first().copied(),
                entries,
                blocks,
                records,
                links,
                root: projection,
            });
        }
    }
    None
}

fn rank(node: &Node, namespace: &str) -> Option<u8> {
    if node.namespace != namespace {
        return None;
    }
    match node.name.as_str() {
        "metadataTypes" => Some(0),
        "metadataStrings" => Some(1),
        "mdxMetadata" => Some(2),
        "futureMetadata" => Some(3),
        "cellMetadata" => Some(4),
        "valueMetadata" => Some(5),
        "extLst" => Some(6),
        _ => None,
    }
}
fn element_end(xml: &str, node: &Node) -> Option<usize> {
    if node.empty {
        return Some(node.head_end);
    }
    // close points at this element's end tag, which has no quoted attributes.
    Some(node.close + xml.get(node.close..)?.find('>')? + 1)
}
fn location(xml: &str, tree: &[Node], root: usize, layout: &Layout, target: u8) -> Option<Anchor> {
    let namespace = &tree[root].namespace;
    let preceding = layout
        .root
        .nodes
        .iter()
        .enumerate()
        .rev()
        .find(|(_, &i)| rank(&tree[i], namespace).is_some_and(|r| r <= target));
    let following = layout
        .root
        .nodes
        .iter()
        .enumerate()
        .find(|(_, &i)| rank(&tree[i], namespace).is_some_and(|r| r > target));
    let lower = preceding.map_or(0, |(i, _)| i + 1);
    let upper = following.map_or(layout.root.nodes.len(), |(i, _)| i);
    if lower > upper {
        return None;
    }
    let mut candidates = Vec::new();
    if let Some((position, &index)) = preceding {
        candidates.push((
            layout.root.depths[position],
            Anchor::Position(element_end(xml, &tree[index])?),
        ));
    }
    if let Some((position, &index)) = following {
        candidates.push((
            layout.root.depths[position],
            Anchor::Position(tree[index].start),
        ));
    }
    for &(offset, branch, depth) in &layout.root.slots {
        if lower <= offset && offset <= upper {
            let anchor = if tree[branch].empty {
                Anchor::EmptyBranch(branch)
            } else {
                Anchor::Position(tree[branch].close)
            };
            candidates.push((depth, anchor));
        }
    }
    Some(
        candidates
            .into_iter()
            .max_by_key(|&(depth, _)| depth)
            .map_or(Anchor::Position(tree[root].close), |(_, at)| at),
    )
}

pub(super) fn append_dynamic(
    xml: &str,
    package: &OpcPackage,
    workbook: &str,
) -> Result<Option<(String, u32)>> {
    let tree = nodes(xml)?;
    if tree[0].children.len() != 1 {
        return Ok(None);
    }
    let root = tree[0].children[0];
    if tree[root].name != "metadata" || !valid_namespace(&tree[root].namespace) {
        return Ok(None);
    }
    let Some(layouts) = layouts(&tree, root) else {
        return Ok(None);
    };
    let Some(maximum) = layouts.iter().map(|l| l.blocks.len()).max() else {
        return Ok(None);
    };
    let Some(cm) = maximum.checked_add(1).and_then(|i| u32::try_from(i).ok()) else {
        return Ok(None);
    };
    let minimum = layouts.iter().map(|l| l.blocks.len()).min().unwrap();
    // Padding may only introduce unreferenced indexes. Every old worksheet
    // link must already resolve even in the shortest stored alternative.
    let Some(rels) = package.part_str(&jade_opc::rels_part_name(workbook))? else {
        return Ok(None);
    };
    for rel in jade_opc::parse_relationships(&rels)?.by_type_suffix("/worksheet") {
        if rel.external {
            continue;
        }
        let part = jade_opc::resolve_target(workbook, &rel.target);
        let Some(sheet) = package.part_str(&part)? else {
            return Ok(None);
        };
        let Some(links) = worksheet_metadata_links(&sheet)? else {
            return Ok(None);
        };
        if links
            .into_iter()
            .any(|link| link.is_none_or(|n| n == 0 || n > minimum))
        {
            return Ok(None);
        }
    }
    let xmlns = format!(" xmlns=\"{}\"", escape(&tree[root].namespace));
    let record = format!("<bk{xmlns}><extLst><ext uri=\"{{BDBB8CDC-FA1E-496E-A857-3C3F30C029C3}}\"><dynamicArrayProperties xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray\" fDynamic=\"1\" fCollapsed=\"0\"/></ext></extLst></bk>");
    let metadata_type = format!("<metadataType{xmlns} name=\"XLDAPR\" minSupportedVersion=\"120000\" copy=\"1\" pasteAll=\"1\" pasteValues=\"1\" merge=\"1\" splitFirst=\"1\" rowColShift=\"1\" clearFormats=\"1\" clearComments=\"1\" assign=\"1\" coerce=\"1\" cellMeta=\"1\"/>");
    let budget = xml.len().saturating_mul(4).max(65536);
    let mut combined = BTreeMap::new();
    let mut inserted = 0usize;
    for layout in layouts {
        for (container, count) in [
            (layout.types, layout.entries.len()),
            (layout.cells, layout.blocks.len()),
            (layout.future, layout.records.len()),
        ] {
            if container.is_some_and(|i| !valid_count(&tree[i], count)) {
                return Ok(None);
            }
        }
        let dynamic: Vec<_> = layout
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, &index)| (tree[index].attr("name") == Some("XLDAPR")).then_some(i + 1))
            .collect();
        if dynamic.len() > 1 {
            return Ok(None);
        }
        let type_index = dynamic.first().copied().unwrap_or(layout.entries.len() + 1);
        for &link in &layout.links {
            let Some(t) = tree[link].index("t") else {
                return Ok(None);
            };
            if t == 0
                || t > layout.entries.len()
                || t == type_index
                    && tree[link]
                        .index("v")
                        .is_none_or(|v| v >= layout.records.len())
            {
                return Ok(None);
            }
        }
        let cell = format!(
            "<bk{xmlns}><rc t=\"{type_index}\" v=\"{}\"/></bk>",
            layout.records.len()
        );
        let repeats = cm as usize - layout.blocks.len();
        if cell.len().checked_mul(repeats).is_none_or(|n| n > budget) {
            return Ok(None);
        }
        // Each pad is a valid pure record, never an empty cell-metadata bk.
        let cells = cell.repeat(repeats);
        let mut splices = Vec::new();
        let mut inserts = BTreeMap::<Anchor, String>::new();
        if dynamic.is_empty() {
            if let Some(index) = layout.types {
                append(
                    xml,
                    &tree[index],
                    layout.entries.len() + 1,
                    &metadata_type,
                    &mut splices,
                )?;
            } else {
                let Some(at) = location(xml, &tree, root, &layout, 0) else {
                    return Ok(None);
                };
                inserts.entry(at).or_default().push_str(&format!(
                    "<metadataTypes{xmlns} count=\"1\">{metadata_type}</metadataTypes>"
                ));
            }
        }
        if let Some(index) = layout.future {
            append(
                xml,
                &tree[index],
                layout.records.len() + 1,
                &record,
                &mut splices,
            )?;
        } else {
            let Some(at) = location(xml, &tree, root, &layout, 3) else {
                return Ok(None);
            };
            inserts.entry(at).or_default().push_str(&format!(
                "<futureMetadata{xmlns} name=\"XLDAPR\" count=\"1\">{record}</futureMetadata>"
            ));
        }
        if let Some(index) = layout.cells {
            append(xml, &tree[index], cm as usize, &cells, &mut splices)?;
        } else {
            let Some(at) = location(xml, &tree, root, &layout, 4) else {
                return Ok(None);
            };
            inserts.entry(at).or_default().push_str(&format!(
                "<cellMetadata{xmlns} count=\"{cm}\">{cells}</cellMetadata>"
            ));
        }
        for (anchor, text) in inserts {
            splices.push(match anchor {
                Anchor::Position(start) => Splice {
                    start,
                    end: start,
                    text,
                },
                Anchor::EmptyBranch(index) => {
                    let node = &tree[index];
                    let mut reader = quick_xml::Reader::from_str(&xml[node.start..node.head_end]);
                    let quick_xml::events::Event::Empty(head) = reader.read_event()? else {
                        return Ok(None);
                    };
                    let head = crate::shift::rewrite(&head, &[], false)?;
                    Splice {
                        start: node.start,
                        end: node.head_end,
                        text: format!("{head}{text}</{}>", node.qualified),
                    }
                }
            });
        }
        for splice in splices {
            let key = (splice.start, splice.end);
            // Different layouts can share a physical container. A shared
            // replacement must agree exactly; conflicting plans stay opaque.
            if let Some(text) = combined.get(&key) {
                if *text != splice.text {
                    return Ok(None);
                }
                continue;
            }
            let Some(total) = inserted
                .checked_add(splice.text.len())
                .filter(|&n| n <= budget)
            else {
                return Ok(None);
            };
            inserted = total;
            combined.insert(key, splice.text);
        }
    }
    let mut end = 0;
    for &(start, stop) in combined.keys() {
        if start < end {
            return Ok(None);
        }
        end = stop;
    }
    let updated = apply(
        xml,
        combined
            .into_iter()
            .map(|((start, end), text)| Splice { start, end, text })
            .collect(),
    );
    let Some(common) =
        jade_xlsx_reader::array_metadata::common_dynamic_array_cell_metadata(&updated)?
    else {
        return Ok(None);
    };
    Ok(Some((updated, common)))
}
