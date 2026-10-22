//! A tagged PDF's structure tree from what the renderer recorded as it drew
//! (`StructureEntry`): the body's paragraphs as `P`, or `H1`–`H6` for the
//! headings the navigation pane lists; tables as `Table`, `TR` and `TD` by
//! their rows and columns; pictures as `Figure` with their alternative
//! text — all in the order they were drawn, which is reading order. The
//! title is the document's (`docProps/core.xml`), and the language the
//! text's.

use std::collections::{BTreeMap, HashMap};

use jade_docx_editor::canvas_renderer::{StructureEntry, StructureKind};
use jade_docx_editor::word_document::WordDocument;
use jade_skia_painter::{PdfTag, PdfTagging};

/// Node ids for the tree's containers (`Document`, `Table`, `TR`), which
/// mark no drawing: past any the renderer gives out.
struct Containers(i32);

impl Containers {
    fn next(&mut self) -> i32 {
        self.0 += 1;
        self.0
    }
}

pub(crate) fn tagging(document: &WordDocument, entries: &[StructureEntry]) -> PdfTagging {
    let headings: HashMap<usize, u8> = document
        .navigation_headings()
        .into_iter()
        .map(|heading| (heading.paragraph_index, heading.level))
        .collect();
    let mut children: HashMap<i32, Vec<&StructureEntry>> = HashMap::new();
    for entry in entries {
        children.entry(entry.parent).or_default().push(entry);
    }
    let mut containers = Containers(1_000_000_000);
    let root = PdfTag {
        kind: "Document".into(),
        node_id: containers.next(),
        alt: None,
        children: build(0, &children, &headings, &mut containers),
    };
    let properties = document.document_properties();
    let title = properties
        .get("Title")
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty());
    let language = Some(if document_is_japanese(document) {
        "ja-JP".to_string()
    } else {
        "en-US".to_string()
    });
    PdfTagging {
        root,
        title,
        language,
    }
}

fn document_is_japanese(document: &WordDocument) -> bool {
    (0..document.paragraph_count().min(200)).any(|index| {
        document.paragraph(index).is_some_and(|paragraph| {
            paragraph.runs.iter().any(|run| {
                run.text
                    .chars()
                    .any(|ch| matches!(ch as u32, 0x3040..=0x30ff | 0x4e00..=0x9fff))
            })
        })
    })
}

fn build(
    parent: i32,
    children: &HashMap<i32, Vec<&StructureEntry>>,
    headings: &HashMap<usize, u8>,
    containers: &mut Containers,
) -> Vec<PdfTag> {
    let Some(list) = children.get(&parent) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut index = 0;
    while index < list.len() {
        let entry = list[index];
        match &entry.kind {
            StructureKind::Paragraph {
                paragraph_index, ..
            } => {
                let kind = match headings.get(paragraph_index) {
                    Some(level) => format!("H{}", (*level).clamp(1, 6)),
                    None => "P".to_string(),
                };
                out.push(PdfTag {
                    kind,
                    node_id: entry.id,
                    alt: None,
                    children: build(entry.id, children, headings, containers),
                });
                index += 1;
            }
            StructureKind::Figure { alt } => {
                out.push(PdfTag {
                    kind: "Figure".into(),
                    node_id: entry.id,
                    alt: alt.clone(),
                    children: build(entry.id, children, headings, containers),
                });
                index += 1;
            }
            StructureKind::Cell { table, .. } => {
                // The table's cells, drawn one after another: rows by their
                // index, cells by their column. A row a page break split is
                // drawn twice; its second part goes inside the first.
                let table = *table;
                let mut rows: BTreeMap<usize, Vec<PdfTag>> = BTreeMap::new();
                let mut seen: HashMap<(usize, usize), usize> = HashMap::new();
                while let Some(cell) = list.get(index) {
                    let StructureKind::Cell {
                        table: cell_table,
                        row,
                        column,
                    } = &cell.kind
                    else {
                        break;
                    };
                    if *cell_table != table {
                        break;
                    }
                    let tag = PdfTag {
                        kind: "TD".into(),
                        node_id: cell.id,
                        alt: None,
                        children: build(cell.id, children, headings, containers),
                    };
                    let cells = rows.entry(*row).or_default();
                    match seen.get(&(*row, *column)) {
                        Some(&at) => cells[at].children.push(PdfTag {
                            kind: "P".into(),
                            ..tag
                        }),
                        None => {
                            seen.insert((*row, *column), cells.len());
                            cells.push(tag);
                        }
                    }
                    index += 1;
                }
                let rows = rows
                    .into_values()
                    .map(|cells| PdfTag {
                        kind: "TR".into(),
                        node_id: containers.next(),
                        alt: None,
                        children: cells,
                    })
                    .collect();
                out.push(PdfTag {
                    kind: "Table".into(),
                    node_id: containers.next(),
                    alt: None,
                    children: rows,
                });
            }
        }
    }
    out
}
