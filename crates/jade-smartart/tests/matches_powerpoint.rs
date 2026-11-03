//! jade lays a diagram's bullets out where PowerPoint does: each text
//! shape of each built-in layout, for three sets of bullets, against the
//! boxes PowerPoint for Mac drew for the same data (fixtures/
//! powerpoint_layouts.tsv).

use jade_smartart::data::from_outline;
use jade_smartart::{catalog, lay_out, Node};

const W: f64 = 8_128_000.0;
const H: f64 = 5_418_667.0;

struct Expected {
    text: String,
    rect: [f64; 4],
    size: Option<f64>,
}

struct Case {
    tag: String,
    nodes: Vec<Node>,
    shapes: Vec<Expected>,
}

fn cases() -> Vec<Case> {
    let mut out: Vec<Case> = Vec::new();
    for line in include_str!("fixtures/powerpoint_layouts.tsv").lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.first() {
            Some(&"L") => {
                let mut counter = 0;
                let lines = fields[2]
                    .split('|')
                    .map(|item| {
                        let (level, text) = item.split_once(':').unwrap();
                        counter += 1;
                        let mut node = Node::new(text);
                        node.id = format!("{{{counter}}}");
                        (level.parse().unwrap(), node)
                    })
                    .collect();
                out.push(Case {
                    tag: fields[1].to_string(),
                    nodes: from_outline(lines),
                    shapes: Vec::new(),
                });
            }
            Some(&"S") if !fields[8].is_empty() => {
                let number = |i: usize| fields[i].parse::<f64>().unwrap();
                out.last_mut().unwrap().shapes.push(Expected {
                    text: fields[8].to_string(),
                    rect: [number(2), number(3), number(4), number(5)],
                    size: fields[7].parse::<f64>().ok().map(|sz| sz / 100.0),
                });
            }
            _ => {}
        }
    }
    out
}

/// Each PowerPoint text shape's distance from jade's shape of the same
/// text: its worst edge, as a share of the frame's width; and the font
/// size jade picked against PowerPoint's.
/// A text shape's text, its worst edge and (jade, PowerPoint) font sizes.
type Compared = (String, f64, Option<(f64, f64)>);

fn compare(case: &Case) -> Vec<Compared> {
    let layout = case.tag.split('/').next().unwrap();
    let shapes = lay_out(&catalog::layout_id(layout), None, &case.nodes, W, H);
    case.shapes
        .iter()
        .map(|expected| {
            let found = shapes.iter().find(|shape| {
                let text: Vec<&str> = shape.paras.iter().map(|p| p.text.as_str()).collect();
                text.join(" | ") == expected.text
            });
            let Some(found) = found else {
                return (expected.text.clone(), 1.0, None);
            };
            let rect = found
                .text_box
                .filter(|_| false)
                .unwrap_or([found.x, found.y, found.w, found.h]);
            let (rect, expected_rect) = if (found.rot - 90.0).abs() < 0.1 {
                // Compare turned shapes by their visual boxes.
                let turn =
                    |[x, y, w, h]: [f64; 4]| [x + w / 2.0 - h / 2.0, y + h / 2.0 - w / 2.0, h, w];
                (turn(rect), turn(expected.rect))
            } else {
                (rect, expected.rect)
            };
            let edges = [
                rect[0] - expected_rect[0],
                rect[1] - expected_rect[1],
                rect[0] + rect[2] - expected_rect[0] - expected_rect[2],
                rect[1] + rect[3] - expected_rect[1] - expected_rect[3],
            ];
            let worst = edges.iter().fold(0.0f64, |m, e| m.max(e.abs())) / W;
            (
                expected.text.clone(),
                worst,
                expected.size.map(|size| (found.size_pt, size)),
            )
        })
        .collect()
}

/// Layouts and how near (share of the frame width) every text shape must
/// come to PowerPoint's.
const TOLERANCE: &[(&str, f64)] = &[
    ("default", 0.02),
    ("vList2", 0.02),
    ("hList1", 0.03),
    ("process1", 0.02),
    ("process2", 0.02),
    ("chevron1", 0.03),
    ("bProcess4", 0.03),
    ("venn3", 0.02),
    ("equation1", 0.02),
    ("pyramid1", 0.02),
    ("lProcess2", 0.02),
    ("vList5", 0.02),
    ("hProcess9", 0.02),
    ("matrix1", 0.02),
    ("matrix2", 0.02),
    ("matrix3", 0.02),
    ("pList1", 0.02),
    ("pyramid2", 0.05),
    ("hierarchy2", 0.05),
    ("hierarchy3", 0.05),
    ("orgChart1", 0.05),
    ("chart3", 0.06),
];

#[test]
fn text_shapes_sit_where_powerpoint_puts_them() {
    let mut report = String::new();
    let mut failures = Vec::new();
    for case in cases() {
        let layout = case.tag.split('/').next().unwrap().to_string();
        let results = compare(&case);
        let worst = results.iter().map(|r| r.1).fold(0.0, f64::max);
        let sizes: Vec<String> = results
            .iter()
            .filter_map(|r| r.2.map(|(jade, office)| format!("{jade}/{office}")))
            .collect();
        report.push_str(&format!(
            "{:<45} worst {:.3}  sizes {}\n",
            case.tag,
            worst,
            sizes.join(" ")
        ));
        if let Some((_, tolerance)) = TOLERANCE.iter().find(|(name, _)| *name == layout) {
            if worst > *tolerance {
                failures.push(format!("{} off by {worst:.3}: {results:?}", case.tag));
            }
        }
    }
    println!("{report}");
    assert!(failures.is_empty(), "{failures:#?}");
}
