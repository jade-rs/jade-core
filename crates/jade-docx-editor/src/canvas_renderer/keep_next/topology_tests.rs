use super::*;
use proptest::prelude::*;

#[derive(Clone, Copy, Debug)]
struct Node {
    // 0 = paragraph, 1 = table, 2 = math.
    kind: u8,
    properties: bool,
    keep: Option<bool>,
    break_before: Option<bool>,
    text: u8,
}

impl Node {
    fn is_paragraph(self) -> bool {
        self.kind == 0
    }

    fn keeps_next(self) -> bool {
        self.properties && self.keep == Some(true)
    }

    fn starts_page(self) -> bool {
        self.properties && self.break_before == Some(true)
    }

    fn contains_break(self) -> bool {
        self.text >= 3
    }

    fn element(self) -> BodyElement {
        if self.kind == 1 {
            return BodyElement::from_table(Table::default());
        }
        if self.kind == 2 {
            return BodyElement::from_math_paragraph(MathParagraph::default());
        }
        let texts: &[&str] = match self.text {
            0 => &[],
            1 => &[""],
            2 => &["日本語🧪", "", "body"],
            3 => &["\x0c", "after"],
            4 => &["before", "", "日本語\x0c🧪"],
            _ => &["before\x0cafter", "\x0c"],
        };
        BodyElement::from_paragraph(Paragraph {
            runs: texts
                .iter()
                .map(|text| Run {
                    text: (*text).into(),
                    properties: None,
                    image: None,
                    text_box: None,
                    math: None,
                    field: None,
                    hyperlink: None,
                    node_id: None,
                    text_node_id: None,
                    revision_node_id: None,
                    comment_reference_ids: Vec::new(),
                    footnote_reference_ids: Vec::new(),
                    endnote_reference_ids: Vec::new(),
                    note_separator: None,
                    has_footnote_ref: false,
                    has_endnote_ref: false,
                    revision: None,
                })
                .collect(),
            properties: self.properties.then(|| ParagraphProperties {
                keep_next: self.keep,
                page_break_before: self.break_before,
                ..Default::default()
            }),
            node_id: None,
            comment_ranges: Vec::new(),
            comment_markers: Vec::new(),
        })
    }
}

// An edge joins two source nodes when the left keeps its successor and the
// right can be entered. An inline break can be entered but is excluded from
// the returned group; notably, that can leave Some(start), an existing result.
// Expectations use only the small input model, never parsed/rendered nodes.
fn expected(nodes: &[Node], start: usize, section_end: usize) -> Option<usize> {
    let links: Vec<_> = nodes
        .windows(2)
        .enumerate()
        .map(|(left, pair)| {
            left + 1 < section_end
                && pair[0].is_paragraph()
                && pair[0].keeps_next()
                && !pair[0].contains_break()
                && pair[1].is_paragraph()
                && !pair[1].starts_page()
        })
        .collect();
    let distance = links.iter().skip(start).take_while(|link| **link).count();
    if distance == 0 {
        None
    } else {
        let last = start + distance;
        Some(last - usize::from(nodes[last].contains_break()))
    }
}

fn check(nodes: &[Node], elements: &[BodyElement], context: &str) {
    for start in 0..=nodes.len() + 1 {
        for section_end in 0..=nodes.len() + 1 {
            assert_eq!(
                group_end(elements, start, section_end),
                expected(nodes, start, section_end),
                "{context}: start={start} section_end={section_end} nodes={nodes:?}"
            );
        }
    }
}

#[test]
fn exhaustive_short_topologies_match_independent_edges() {
    let ordinary = Node {
        kind: 0,
        properties: true,
        keep: Some(false),
        break_before: None,
        text: 2,
    };
    let variants = [
        ordinary,
        Node {
            keep: Some(true),
            ..ordinary
        },
        Node {
            text: 4,
            ..ordinary
        },
        Node {
            keep: Some(true),
            text: 3,
            ..ordinary
        },
        Node {
            keep: Some(true),
            break_before: Some(true),
            ..ordinary
        },
        Node {
            kind: 1,
            ..ordinary
        },
        Node {
            properties: false,
            keep: Some(true),
            ..ordinary
        },
    ];
    // Every sequence through length four, every start and section boundary.
    for length in 0..=4 {
        for mut code in 0..variants.len().pow(length as u32) {
            let nodes: Vec<_> = (0..length)
                .map(|_| {
                    let node = variants[code % variants.len()];
                    code /= variants.len();
                    node
                })
                .collect();
            let elements: Vec<_> = nodes.iter().map(|node| node.element()).collect();
            check(&nodes, &elements, "exhaustive topology");
        }
    }
}

#[test]
fn long_groups_retain_every_kind_of_boundary_at_every_position() {
    let linked = Node {
        kind: 0,
        properties: true,
        keep: Some(true),
        break_before: None,
        text: 2,
    };
    for position in 0..32 {
        for replacement in [
            Node {
                keep: Some(false),
                ..linked
            },
            Node {
                keep: None,
                ..linked
            },
            Node {
                properties: false,
                ..linked
            },
            Node { text: 5, ..linked },
            Node {
                break_before: Some(true),
                ..linked
            },
            Node { kind: 1, ..linked },
            Node { kind: 2, ..linked },
        ] {
            let mut nodes = vec![linked; 32];
            nodes[position] = replacement;
            let elements: Vec<_> = nodes.iter().map(|node| node.element()).collect();
            check(&nodes, &elements, "long group boundary");
        }
    }
}

fn node_strategy() -> impl Strategy<Value = Node> {
    (
        prop_oneof![8 => Just(0u8), 1 => Just(1), 1 => Just(2)],
        prop_oneof![3 => Just(true), 1 => Just(false)],
        prop_oneof![4 => Just(Some(true)), 1 => Just(Some(false)), 1 => Just(None)],
        prop_oneof![1 => Just(Some(true)), 3 => Just(Some(false)), 3 => Just(None)],
        prop_oneof![6 => 0u8..3, 2 => 3u8..6],
    )
        .prop_map(|(kind, properties, keep, break_before, text)| Node {
            kind,
            properties,
            keep,
            break_before,
            text,
        })
}

#[derive(Clone, Debug)]
enum Edit {
    Insert(usize, Node),
    Replace(usize, Node),
    Remove(usize),
    Clear,
}

fn edit_strategy() -> impl Strategy<Value = Edit> {
    prop_oneof![
        4 => (0usize..24, node_strategy()).prop_map(|(at, node)| Edit::Insert(at, node)),
        4 => (0usize..24, node_strategy()).prop_map(|(at, node)| Edit::Replace(at, node)),
        2 => (0usize..24).prop_map(Edit::Remove),
        1 => Just(Edit::Clear),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]
    #[test]
    fn edited_topologies_match_model_after_every_operation(
        initial in prop::collection::vec(node_strategy(), 0..12),
        edits in prop::collection::vec(edit_strategy(), 1..65),
    ) {
        let mut nodes = initial;
        let mut elements: Vec<_> = nodes.iter().map(|node| node.element()).collect();
        check(&nodes, &elements, "initial topology");
        for (step, edit) in edits.into_iter().enumerate() {
            let context = format!("step={step} edit={edit:?}");
            match edit {
                Edit::Insert(at, node) => {
                    let at = at.min(nodes.len());
                    nodes.insert(at, node);
                    elements.insert(at, node.element());
                }
                Edit::Replace(at, node) if at < nodes.len() => {
                    nodes[at] = node;
                    elements[at] = node.element();
                }
                Edit::Remove(at) if at < nodes.len() => {
                    nodes.remove(at);
                    elements.remove(at);
                }
                Edit::Clear => {
                    nodes.clear();
                    elements.clear();
                }
                _ => {},
            }
            check(&nodes, &elements, &context);
        }
    }
}
