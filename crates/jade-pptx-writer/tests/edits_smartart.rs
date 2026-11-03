//! SmartArt as the SmartArt のデザイン and 書式 tabs edit it: a diagram
//! inserted, its bullets, layout, colours, style and one shape's look
//! changed, a list turned into a diagram and a diagram into shapes — each
//! readable again, with PowerPoint's built-ins left to the parts jade
//! leaves out, and undoable.

use jade_opc::{parse_relationships, OpcPackage};
use jade_pptx_reader::{read_deck, ShapeItem, Transform};
use jade_pptx_writer::{smartart, EditSession, SmartArtEdit, TextFormat};
use jade_smartart::{Node, ShapeFormat};

const DECK: &[u8] = include_bytes!("../../../public/fixtures/sample-deck.pptx");

fn frame() -> Transform {
    Transform {
        offset_emu: (914_400, 914_400),
        extent_emu: (6_096_000, 3_429_000),
        rotation_60000ths: 0,
        flip_h: false,
        flip_v: false,
    }
}

fn bullets() -> Vec<Node> {
    vec![
        Node::with("Plan", vec![Node::new("Scope"), Node::new("Budget")]),
        Node::new("Build"),
        Node::new("Ship"),
    ]
}

fn texts(bytes: &[u8], id: u32) -> Vec<String> {
    let deck = read_deck(bytes).unwrap();
    let group = deck.slides[0]
        .content
        .shapes
        .iter()
        .find_map(|item| match item {
            ShapeItem::Group(group) if group.id == Some(id) => Some(group.clone()),
            _ => None,
        })
        .expect("the diagram reads as a group");
    group
        .children
        .iter()
        .filter_map(|item| match item {
            ShapeItem::Shape(shape) => shape.text.as_ref(),
            _ => None,
        })
        .flat_map(|body| body.paragraphs.iter())
        .map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>())
        .filter(|text| !text.is_empty())
        .collect()
}

fn part(bytes: &[u8], name: &str) -> Option<String> {
    OpcPackage::open(bytes).unwrap().part_str(name).unwrap()
}

fn drawing(bytes: &[u8]) -> String {
    let package = OpcPackage::open(bytes).unwrap();
    let name = package
        .part_names()
        .iter()
        .find(|name| name.starts_with("ppt/diagrams/drawing"))
        .cloned()
        .expect("a drawing part");
    package.required_part_str(&name).unwrap()
}

#[test]
fn an_inserted_diagram_is_its_data_and_drawing_and_reads_back() {
    let mut session = EditSession::open(DECK).unwrap();
    let id = session
        .insert_smartart(0, frame(), "process1", &bullets())
        .unwrap();
    let bytes = session.bytes().to_vec();
    let shown = texts(&bytes, id);
    for word in ["Plan", "Scope", "Budget", "Build", "Ship"] {
        assert!(shown.iter().any(|t| t == word), "{word} in {shown:?}");
    }
    let slide = part(&bytes, "ppt/slides/slide1.xml").unwrap();
    assert!(slide.contains(":lo=\"\""), "no layout part of its own");
    let types = part(&bytes, "[Content_Types].xml").unwrap();
    assert!(types.contains("diagramData+xml") && types.contains("diagramDrawing+xml"));
    let info = smartart(&bytes, 0, id).unwrap().expect("a diagram");
    assert!(info.layout.ends_with("/process1"));
    assert!(info.laid_out_by_jade);
    assert_eq!(info.nodes.len(), 3);
    assert!(info
        .shapes
        .iter()
        .any(|shape| shape.nodes.contains(&info.nodes[1].id) && shape.rect.2 > 0));
    assert!(session.undo());
    assert_eq!(session.bytes(), DECK);
}

#[test]
fn the_text_pane_adds_removes_promotes_and_demotes_bullets() {
    let mut session = EditSession::open(DECK).unwrap();
    let id = session
        .insert_smartart(0, frame(), "default", &bullets())
        .unwrap();
    let mut nodes = smartart(session.bytes(), 0, id).unwrap().unwrap().nodes;
    // Promote "Budget" to a bullet of its own, demote "Ship" under "Build",
    // add "Review", remove "Scope".
    let budget = nodes[0].children.remove(1);
    nodes[0].children.remove(0);
    nodes.insert(1, budget);
    let ship = nodes.remove(3);
    nodes[2].children.push(ship);
    nodes.push(Node::new("Review"));
    assert!(session
        .edit_smartart(0, id, &SmartArtEdit::Nodes(nodes))
        .unwrap());
    let info = smartart(session.bytes(), 0, id).unwrap().unwrap();
    let outline: Vec<(usize, String)> = jade_smartart::data::outline(&info.nodes)
        .into_iter()
        .map(|(level, node)| (level, node.text.clone()))
        .collect();
    assert_eq!(
        outline,
        vec![
            (0, "Plan".to_string()),
            (0, "Budget".to_string()),
            (0, "Build".to_string()),
            (1, "Ship".to_string()),
            (0, "Review".to_string()),
        ]
    );
    let shown = texts(session.bytes(), id);
    assert!(!shown.iter().any(|t| t == "Scope"));
    assert!(shown.iter().any(|t| t == "Review"));
}

#[test]
fn layout_colours_and_style_name_built_ins_and_redraw() {
    let mut session = EditSession::open(DECK).unwrap();
    let id = session
        .insert_smartart(0, frame(), "default", &bullets())
        .unwrap();
    session
        .edit_smartart(0, id, &SmartArtEdit::Layout("cycle2".into()))
        .unwrap();
    assert!(drawing(session.bytes()).contains("prst=\"ellipse\""));
    session
        .edit_smartart(0, id, &SmartArtEdit::Colors("colorful1".into()))
        .unwrap();
    assert!(drawing(session.bytes()).contains("val=\"accent2\""));
    session
        .edit_smartart(0, id, &SmartArtEdit::Style("3d1".into()))
        .unwrap();
    assert!(drawing(session.bytes()).contains("<a:bevelT"));
    let info = smartart(session.bytes(), 0, id).unwrap().unwrap();
    assert!(info.layout.ends_with("/cycle2"));
    assert!(info.colors.ends_with("/colorful1"));
    assert!(info.style.ends_with("/3d1"));
    let slide = part(session.bytes(), "ppt/slides/slide1.xml").unwrap();
    assert!(slide.contains(":lo=\"\"") && slide.contains(":cs=\"\"") && slide.contains(":qs=\"\""));
}

#[test]
fn one_shape_keeps_a_format_of_its_own_until_the_graphic_is_reset() {
    let mut session = EditSession::open(DECK).unwrap();
    let id = session
        .insert_smartart(0, frame(), "process1", &bullets())
        .unwrap();
    let node = smartart(session.bytes(), 0, id).unwrap().unwrap().nodes[1]
        .id
        .clone();
    let format = ShapeFormat {
        fill: Some("FF0000".into()),
        line: Some("0000FF".into()),
        preset: Some("ellipse".into()),
        ..ShapeFormat::default()
    };
    session
        .edit_smartart(
            0,
            id,
            &SmartArtEdit::Format {
                node: node.clone(),
                format: format.clone(),
            },
        )
        .unwrap();
    let drawn = drawing(session.bytes());
    assert!(drawn.contains("val=\"FF0000\"") && drawn.contains("prst=\"ellipse\""));
    let info = smartart(session.bytes(), 0, id).unwrap().unwrap();
    assert_eq!(info.nodes[1].format, format);
    // The formatting rides on a presentation point PowerPoint honours.
    let package = OpcPackage::open(session.bytes()).unwrap();
    let data_name = package
        .part_names()
        .iter()
        .find(|n| n.starts_with("ppt/diagrams/data"))
        .unwrap()
        .clone();
    let data = package.required_part_str(&data_name).unwrap();
    assert!(data.contains("type=\"pres\"") && data.contains("type=\"presOf\""));
    // Editing the bullets keeps it.
    let mut nodes = info.nodes.clone();
    nodes[1].text = "Make".into();
    session
        .edit_smartart(0, id, &SmartArtEdit::Nodes(nodes))
        .unwrap();
    assert!(drawing(session.bytes()).contains("val=\"FF0000\""));
    session
        .edit_smartart(0, id, &SmartArtEdit::ResetGraphic)
        .unwrap();
    assert!(!drawing(session.bytes()).contains("val=\"FF0000\""));
    let info = smartart(session.bytes(), 0, id).unwrap().unwrap();
    assert!(info.nodes.iter().all(|node| node.format.is_empty()));
}

#[test]
fn a_diagram_becomes_a_group_of_shapes_and_its_parts_go() {
    let mut session = EditSession::open(DECK).unwrap();
    let id = session
        .insert_smartart(0, frame(), "process1", &bullets())
        .unwrap();
    let group = session.convert_smartart_to_shapes(0, id).unwrap();
    let bytes = session.bytes().to_vec();
    let deck = read_deck(&bytes).unwrap();
    let converted = deck.slides[0]
        .content
        .shapes
        .iter()
        .find_map(|item| match item {
            ShapeItem::Group(g) if g.id == Some(group) => Some(g.clone()),
            _ => None,
        })
        .expect("a group");
    assert!(converted.diagram_data_rel_id.is_none());
    assert_eq!(converted.children.len(), 5, "three boxes and two arrows");
    let package = OpcPackage::open(&bytes).unwrap();
    assert!(!package
        .part_names()
        .iter()
        .any(|n| n.starts_with("ppt/diagrams/")));
    let rels = parse_relationships(
        &package
            .required_part_str("ppt/slides/_rels/slide1.xml.rels")
            .unwrap(),
    )
    .unwrap();
    assert!(rels.by_type_suffix("/diagramData").is_empty());
    let types = package.required_part_str("[Content_Types].xml").unwrap();
    assert!(!types.contains("/ppt/diagrams/"));
    assert!(smartart(&bytes, 0, group).unwrap().is_none());
}

#[test]
fn a_bulleted_list_becomes_a_diagram_in_its_place() {
    let mut session = EditSession::open(DECK).unwrap();
    let text = session
        .insert_text_box(0, frame(), "Plan\nBuild\nShip", &TextFormat::default())
        .unwrap();
    let id = session.convert_text_to_smartart(0, text, "vList2").unwrap();
    let info = smartart(session.bytes(), 0, id).unwrap().unwrap();
    let words: Vec<&str> = info.nodes.iter().map(|n| n.text.as_str()).collect();
    assert_eq!(words, ["Plan", "Build", "Ship"]);
    let deck = read_deck(session.bytes()).unwrap();
    assert!(!deck.slides[0]
        .content
        .shapes
        .iter()
        .any(|item| matches!(item, ShapeItem::Shape(s) if s.id == Some(text))));
}

#[test]
fn a_picture_fills_a_bullets_picture_shape() {
    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(8, 8, image::Rgb([0, 128, 255])))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let mut session = EditSession::open(DECK).unwrap();
    let id = session
        .insert_smartart(0, frame(), "pList1", &bullets())
        .unwrap();
    let node = smartart(session.bytes(), 0, id).unwrap().unwrap().nodes[0]
        .id
        .clone();
    session.set_smartart_picture(0, id, &node, &png).unwrap();
    let deck = read_deck(session.bytes()).unwrap();
    let group = deck.slides[0]
        .content
        .shapes
        .iter()
        .find_map(|item| match item {
            ShapeItem::Group(g) if g.id == Some(id) => Some(g.clone()),
            _ => None,
        })
        .unwrap();
    assert!(group.children.iter().any(|item| matches!(
        item,
        ShapeItem::Shape(shape) if shape.image_part.as_deref().is_some_and(|p| p.starts_with("ppt/media/"))
    )), "{:?}", group.children);
}
