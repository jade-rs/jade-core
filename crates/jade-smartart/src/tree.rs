//! The SmartArt のデザイン tab's 図形の作成 group on the bullet tree: add a
//! shape after, before, above or below one, an assistant, promote, demote,
//! move up or down, delete — each worked on the text pane's outline, as
//! PowerPoint works them.

use crate::data::{from_outline, outline, Node, NodeKind};

/// Where 図形の追加 puts the new shape, relative to the selected one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// 後に図形を追加: the next sibling.
    After,
    /// 前に図形を追加: the previous sibling.
    Before,
    /// 上に図形を追加: a new parent, in its place, over it.
    Above,
    /// 下に図形を追加: its last child.
    Below,
    /// アシスタントの追加.
    Assistant,
}

type Lines = Vec<(usize, Node)>;

fn lines(nodes: &[Node]) -> Lines {
    outline(nodes)
        .into_iter()
        .map(|(level, node)| {
            let mut node = node.clone();
            node.children.clear();
            (level, node)
        })
        .collect()
}

/// The line of `id` and the end (exclusive) of its subtree.
fn span(lines: &Lines, id: &str) -> Option<(usize, usize)> {
    let start = lines.iter().position(|(_, node)| node.id == id)?;
    let level = lines[start].0;
    let end = lines[start + 1..]
        .iter()
        .position(|(l, _)| *l <= level)
        .map_or(lines.len(), |offset| start + 1 + offset);
    Some((start, end))
}

/// Adds `new` at `place` relative to bullet `id`; with no `id` (nothing
/// selected), after the last top-level bullet.
pub fn add(nodes: &[Node], id: Option<&str>, place: Place, mut new: Node) -> Vec<Node> {
    let mut lines = lines(nodes);
    let Some((start, end)) = id.and_then(|id| span(&lines, id)) else {
        lines.push((0, new));
        return from_outline(lines);
    };
    let level = lines[start].0;
    match place {
        Place::After => lines.insert(end, (level, new)),
        Place::Before => lines.insert(start, (level, new)),
        Place::Above => {
            for line in &mut lines[start..end] {
                line.0 += 1;
            }
            lines.insert(start, (level, new));
        }
        Place::Below => lines.insert(end, (level + 1, new)),
        Place::Assistant => {
            new.kind = NodeKind::Assistant;
            // Assistants come first among the children.
            lines.insert(start + 1, (level + 1, new));
        }
    }
    from_outline(lines)
}

/// レベル上げ: bullet `id` and its subtree one level up; the bullets after
/// it under the same parent become its children.
pub fn promote(nodes: &[Node], id: &str) -> Vec<Node> {
    let mut lines = lines(nodes);
    let Some((start, end)) = span(&lines, id) else {
        return nodes.to_vec();
    };
    if lines[start].0 == 0 {
        return nodes.to_vec();
    }
    for line in &mut lines[start..end] {
        line.0 -= 1;
    }
    from_outline(lines)
}

/// レベル下げ: bullet `id` and its subtree one level down, under the bullet
/// before it; the first bullet of a level cannot go down.
pub fn demote(nodes: &[Node], id: &str) -> Vec<Node> {
    let mut lines = lines(nodes);
    let Some((start, end)) = span(&lines, id) else {
        return nodes.to_vec();
    };
    let level = lines[start].0;
    let has_previous_sibling = lines[..start]
        .iter()
        .rev()
        .take_while(|(l, _)| *l >= level)
        .any(|(l, _)| *l == level);
    if !has_previous_sibling {
        return nodes.to_vec();
    }
    for line in &mut lines[start..end] {
        line.0 += 1;
    }
    from_outline(lines)
}

/// 上へ移動 / 下へ移動: bullet `id` with its subtree swapped with the
/// sibling before (`up`) or after it.
pub fn move_sibling(nodes: &[Node], id: &str, up: bool) -> Vec<Node> {
    fn walk(nodes: &mut [Node], id: &str, up: bool) -> bool {
        if let Some(index) = nodes.iter().position(|node| node.id == id) {
            let other = if up {
                index.checked_sub(1)
            } else {
                Some(index + 1).filter(|i| *i < nodes.len())
            };
            if let Some(other) = other {
                nodes.swap(index, other);
            }
            return true;
        }
        nodes
            .iter_mut()
            .any(|node| walk(&mut node.children, id, up))
    }
    let mut out = nodes.to_vec();
    walk(&mut out, id, up);
    out
}

/// Deletes bullet `id`; its children take its place.
pub fn remove(nodes: &[Node], id: &str) -> Vec<Node> {
    fn walk(nodes: &mut Vec<Node>, id: &str) -> bool {
        if let Some(index) = nodes.iter().position(|node| node.id == id) {
            let removed = nodes.remove(index);
            for (offset, child) in removed.children.into_iter().enumerate() {
                nodes.insert(index + offset, child);
            }
            return true;
        }
        nodes.iter_mut().any(|node| walk(&mut node.children, id))
    }
    let mut out = nodes.to_vec();
    walk(&mut out, id);
    out
}

/// The text pane: bullets as (level, text) lines, edited as a whole — a
/// line's text, lines added, removed, indented — mapped back onto the
/// bullets: lines keep the ids of the bullets they were, in order, as far
/// as the texts allow.
pub fn from_text_pane(nodes: &[Node], text: &[(usize, String)]) -> Vec<Node> {
    let old = lines(nodes);
    let mut used = vec![false; old.len()];
    let mut out = Vec::new();
    let mut cursor = 0;
    for (level, line) in text {
        // The same text further on keeps its bullet; otherwise the next
        // unused bullet in order does (a line edited in place).
        let same = (cursor..old.len()).find(|&i| !used[i] && old[i].1.text == *line);
        let pick = same.or_else(|| {
            (cursor..old.len())
                .find(|&i| !used[i] && !text.iter().any(|(_, t)| t == &old[i].1.text))
        });
        let node = match pick {
            Some(i) => {
                used[i] = true;
                cursor = i + 1;
                let mut node = old[i].1.clone();
                node.text = line.clone();
                node
            }
            None => Node::new(line.clone()),
        };
        out.push((*level, node));
    }
    from_outline(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Vec<Node> {
        let mut nodes = vec![
            Node::with("A", vec![Node::new("A1"), Node::new("A2")]),
            Node::new("B"),
        ];
        let mut n = 0;
        fn number(nodes: &mut [Node], n: &mut usize) {
            for node in nodes {
                node.id = node.text.clone();
                *n += 1;
                number(&mut node.children, n);
            }
        }
        number(&mut nodes, &mut n);
        nodes
    }

    fn show(nodes: &[Node]) -> Vec<(usize, String)> {
        outline(nodes)
            .into_iter()
            .map(|(level, node)| (level, node.text.clone()))
            .collect()
    }

    fn l(items: &[(usize, &str)]) -> Vec<(usize, String)> {
        items.iter().map(|(l, t)| (*l, t.to_string())).collect()
    }

    #[test]
    fn adds_after_before_above_below_and_assistants() {
        let t = tree();
        assert_eq!(
            show(&add(&t, Some("A1"), Place::After, Node::new("N"))),
            l(&[(0, "A"), (1, "A1"), (1, "N"), (1, "A2"), (0, "B")])
        );
        assert_eq!(
            show(&add(&t, Some("A"), Place::After, Node::new("N"))),
            l(&[(0, "A"), (1, "A1"), (1, "A2"), (0, "N"), (0, "B")])
        );
        assert_eq!(
            show(&add(&t, Some("B"), Place::Before, Node::new("N"))),
            l(&[(0, "A"), (1, "A1"), (1, "A2"), (0, "N"), (0, "B")])
        );
        assert_eq!(
            show(&add(&t, Some("A"), Place::Above, Node::new("N"))),
            l(&[(0, "N"), (1, "A"), (2, "A1"), (2, "A2"), (0, "B")])
        );
        assert_eq!(
            show(&add(&t, Some("B"), Place::Below, Node::new("N"))),
            l(&[(0, "A"), (1, "A1"), (1, "A2"), (0, "B"), (1, "N")])
        );
        let with_assistant = add(&t, Some("A"), Place::Assistant, Node::new("N"));
        assert_eq!(with_assistant[0].children[0].kind, NodeKind::Assistant);
    }

    #[test]
    fn promotes_demotes_moves_and_removes() {
        let t = tree();
        assert_eq!(
            show(&promote(&t, "A1")),
            l(&[(0, "A"), (0, "A1"), (1, "A2"), (0, "B")])
        );
        assert_eq!(
            show(&demote(&t, "B")),
            l(&[(0, "A"), (1, "A1"), (1, "A2"), (1, "B")])
        );
        assert_eq!(show(&demote(&t, "A1")), show(&t), "the first child stays");
        assert_eq!(
            show(&move_sibling(&t, "B", true)),
            l(&[(0, "B"), (0, "A"), (1, "A1"), (1, "A2")])
        );
        assert_eq!(show(&remove(&t, "A")), l(&[(0, "A1"), (0, "A2"), (0, "B")]));
    }

    #[test]
    fn the_text_pane_keeps_ids_of_lines_it_keeps() {
        let t = tree();
        let edited = from_text_pane(&t, &l(&[(0, "A"), (1, "A1 changed"), (0, "B"), (1, "C")]));
        let ids: Vec<(String, String)> = outline(&edited)
            .into_iter()
            .map(|(_, node)| (node.text.clone(), node.id.clone()))
            .collect();
        assert_eq!(ids[0], ("A".into(), "A".into()));
        assert_eq!(ids[1], ("A1 changed".into(), "A1".into()));
        assert_eq!(ids[2], ("B".into(), "B".into()));
        assert_eq!(ids[3].1, "", "a new line is a new bullet");
    }
}
