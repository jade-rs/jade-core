//! The data part (`dgm:dataModel`): the points of a diagram — its text,
//! one point per bullet of the text pane — and the connections that nest
//! them, plus the presentation points PowerPoint keeps per shape, where a
//! shape's own formatting lives.
//!
//! [`DataModel`] edits the part in place: [`DataModel::set_nodes`] keeps
//! every point that survives with its id and its markup, so what PowerPoint
//! wrote about a bullet (its run formatting, its presentation) outlives an
//! edit made here.

use std::collections::{HashMap, HashSet};

use crate::xml::{self, Element, Node as XmlNode};

pub const DGM_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/diagram";
pub const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
pub const R_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub const DSP_NS: &str = "http://schemas.microsoft.com/office/drawing/2008/diagram";
pub const LAYOUT_PREFIX: &str = "urn:microsoft.com/office/officeart/2005/8/layout/";
pub const COLORS_PREFIX: &str = "urn:microsoft.com/office/officeart/2005/8/colors/";
pub const STYLE_PREFIX: &str = "urn:microsoft.com/office/officeart/2005/8/quickstyle/";

/// What a bullet is: an ordinary point, or an assistant (組織図's アシスタント),
/// which hangs off its manager's line rather than among the staff.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NodeKind {
    #[default]
    Node,
    Assistant,
}

/// The formatting one shape of a diagram has of its own — what 図形の書式設定
/// on a single SmartArt shape sets — over what the colours and style give.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ShapeFormat {
    /// `RRGGBB` fill.
    pub fill: Option<String>,
    /// `RRGGBB` outline.
    pub line: Option<String>,
    /// `RRGGBB` text.
    pub text: Option<String>,
    /// Another preset shape (図形の変更).
    pub preset: Option<String>,
    /// The relationship (of the data part) of a picture filling the shape.
    pub picture: Option<String>,
}

impl ShapeFormat {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// One bullet of the text pane and the bullets under it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Node {
    /// The point's `modelId`; empty for a bullet not yet in the data.
    pub id: String,
    pub kind: NodeKind,
    /// The bullet's text, paragraphs joined by `\n`.
    pub text: String,
    pub format: ShapeFormat,
    pub children: Vec<Node>,
}

impl Node {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    pub fn with(text: impl Into<String>, children: Vec<Node>) -> Self {
        Self {
            text: text.into(),
            children,
            ..Self::default()
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: NodeKind::Assistant,
            ..Self::default()
        }
    }

    /// Bullets in this subtree, this one included.
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(Node::count).sum::<usize>()
    }
}

/// The bullets as the text pane lists them: each with its level.
pub fn outline(nodes: &[Node]) -> Vec<(usize, &Node)> {
    fn walk<'a>(nodes: &'a [Node], level: usize, out: &mut Vec<(usize, &'a Node)>) {
        for node in nodes {
            out.push((level, node));
            walk(&node.children, level + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(nodes, 0, &mut out);
    out
}

/// The tree the text pane's lines make: a line's level puts it under the
/// last line one level up; a jump of more than one level is one level.
pub fn from_outline(lines: Vec<(usize, Node)>) -> Vec<Node> {
    fn insert(nodes: &mut Vec<Node>, level: usize, node: Node) {
        if level == 0 || nodes.is_empty() {
            nodes.push(node);
        } else {
            let last = nodes.last_mut().expect("not empty");
            insert(&mut last.children, level - 1, node);
        }
    }
    let mut nodes = Vec::new();
    for (level, mut node) in lines {
        node.children.clear();
        insert(&mut nodes, level, node);
    }
    nodes
}

/// A `dgm:dataModel`, edited in place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataModel {
    root: Element,
}

fn guid(seed: u64) -> String {
    // Office's model ids are GUIDs; jade makes them from a counter mixed
    // with the time it starts, unique within a part, which is all that is
    // needed.
    let mixed = seed
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .rotate_left(17)
        .wrapping_add(0xD1B5_4A32_D192_ED03);
    format!(
        "{{{:08X}-{:04X}-4{:03X}-8{:03X}-{:012X}}}",
        (mixed >> 32) as u32,
        (mixed >> 16) as u16,
        (mixed >> 4) as u16 & 0xFFF,
        (seed as u16) & 0xFFF,
        mixed & 0xFFFF_FFFF_FFFF
    )
}

fn empty_text() -> Element {
    Element::new("dgm:t")
        .with_child(Element::new("a:bodyPr"))
        .with_child(Element::new("a:lstStyle"))
        .with_child(
            Element::new("a:p").with_child(Element::new("a:endParaRPr").with_attr("lang", "ja-JP")),
        )
}

fn point(id: &str, kind: Option<&str>) -> Element {
    let mut pt = Element::new("dgm:pt").with_attr("modelId", id);
    if let Some(kind) = kind {
        pt.set_attr("type", kind);
    }
    pt
}

/// The text of a `dgm:t` (or any text body): paragraphs joined by `\n`.
pub fn body_text(body: &Element) -> String {
    let mut out = String::new();
    for (index, paragraph) in body.children_named("p").enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let mut runs = Vec::new();
        paragraph.descendants("t", &mut runs);
        for run in runs {
            out.push_str(&run.text());
        }
    }
    out
}

/// `body` with its paragraphs replaced by `text`'s lines, each run taking
/// the first run's properties so a bullet keeps its font when retyped.
pub fn set_body_text(body: &mut Element, text: &str) {
    let mut first_rpr = None;
    let mut first_ppr = None;
    for paragraph in body.children_named("p") {
        if first_ppr.is_none() {
            first_ppr = paragraph.child("pPr").cloned();
        }
        if let Some(run) = paragraph.child("r") {
            first_rpr = run.child("rPr").cloned();
            break;
        }
        if first_rpr.is_none() {
            first_rpr = paragraph.child("endParaRPr").cloned().map(|mut end| {
                end.name = "a:rPr".to_string();
                end
            });
        }
    }
    let rpr = first_rpr.unwrap_or_else(|| Element::new("a:rPr").with_attr("lang", "ja-JP"));
    body.remove_children("p");
    if body.child("bodyPr").is_none() {
        body.children
            .insert(0, XmlNode::Element(Element::new("a:bodyPr")));
    }
    if body.child("lstStyle").is_none() {
        let at = body
            .children
            .iter()
            .position(|node| matches!(node, XmlNode::Element(e) if e.local() == "bodyPr"))
            .map_or(0, |index| index + 1);
        body.children
            .insert(at, XmlNode::Element(Element::new("a:lstStyle")));
    }
    for line in text.split('\n') {
        let mut paragraph = Element::new("a:p");
        if let Some(ppr) = &first_ppr {
            paragraph.children.push(XmlNode::Element(ppr.clone()));
        }
        if line.is_empty() {
            let mut end = rpr.clone();
            end.name = "a:endParaRPr".to_string();
            paragraph.children.push(XmlNode::Element(end));
        } else {
            let mut t = Element::new("a:t");
            t.children.push(XmlNode::Text(line.to_string()));
            paragraph.children.push(XmlNode::Element(
                Element::new("a:r").with_child(rpr.clone()).with_child(t),
            ));
        }
        body.children.push(XmlNode::Element(paragraph));
    }
}

/// `RRGGBB` of a fill/line container's `a:solidFill/a:srgbClr`.
fn solid_rgb(container: &Element) -> Option<String> {
    container
        .child("solidFill")?
        .child("srgbClr")?
        .attr("val")
        .map(str::to_string)
}

fn solid_fill(rgb: &str) -> Element {
    Element::new("a:solidFill").with_child(Element::new("a:srgbClr").with_attr("val", rgb))
}

/// The formatting a presentation point's `dgm:spPr` and `dgm:t` carry.
fn read_format(pres: &Element) -> ShapeFormat {
    let mut format = ShapeFormat::default();
    if let Some(sp_pr) = pres.child("spPr") {
        format.fill = solid_rgb(sp_pr);
        format.line = sp_pr.child("ln").and_then(solid_rgb);
        format.preset = sp_pr
            .child("prstGeom")
            .and_then(|geom| geom.attr("prst"))
            .map(str::to_string);
        format.picture = sp_pr
            .child("blipFill")
            .and_then(|fill| fill.child("blip"))
            .and_then(|blip| blip.attr("embed"))
            .map(str::to_string);
    }
    format
}

/// `sp_pr` (a `dgm:spPr`) set to `format`'s fill, line and geometry, the
/// rest of what it held kept.
fn write_format(sp_pr: &mut Element, format: &ShapeFormat) {
    // DrawingML order: xfrm, geometry, fill, ln, effects…
    let keep: Vec<XmlNode> = sp_pr
        .children
        .drain(..)
        .filter(|node| match node {
            XmlNode::Element(element) => !matches!(
                element.local(),
                "prstGeom"
                    | "custGeom"
                    | "solidFill"
                    | "noFill"
                    | "gradFill"
                    | "blipFill"
                    | "pattFill"
                    | "grpFill"
            ),
            _ => true,
        })
        .collect();
    let mut ordered = Vec::new();
    let mut rest = Vec::new();
    for node in keep {
        match &node {
            XmlNode::Element(element) if element.local() == "xfrm" => ordered.push(node),
            _ => rest.push(node),
        }
    }
    if let Some(preset) = &format.preset {
        ordered.push(XmlNode::Element(
            Element::new("a:prstGeom")
                .with_attr("prst", preset)
                .with_child(Element::new("a:avLst")),
        ));
    }
    if let Some(picture) = &format.picture {
        ordered.push(XmlNode::Element(
            Element::new("a:blipFill")
                .with_child(Element::new("a:blip").with_attr("r:embed", picture))
                .with_child(Element::new("a:srcRect"))
                .with_child(Element::new("a:stretch").with_child(Element::new("a:fillRect"))),
        ));
    } else if let Some(fill) = &format.fill {
        ordered.push(XmlNode::Element(solid_fill(fill)));
    }
    // The outline is its own element among the rest.
    if let Some(line) = &format.line {
        rest.retain(|node| !matches!(node, XmlNode::Element(e) if e.local() == "ln"));
        rest.insert(
            0,
            XmlNode::Element(Element::new("a:ln").with_child(solid_fill(line))),
        );
    } else {
        // No outline colour of its own: the style's shows again.
        for node in &mut rest {
            if let XmlNode::Element(ln) = node {
                if ln.local() == "ln" {
                    ln.children.retain(
                        |child| !matches!(child, XmlNode::Element(e) if e.local() == "solidFill"),
                    );
                }
            }
        }
        rest.retain(|node| {
            !matches!(node, XmlNode::Element(e)
                if e.local() == "ln" && e.children.is_empty() && e.attrs.is_empty())
        });
    }
    ordered.extend(rest);
    sp_pr.children = ordered;
}

/// What the tree of points looks like: each point's parent and order.
type Shape = Vec<(String, String, NodeKind)>;

impl DataModel {
    /// Reads a data part.
    pub fn parse(xml: &str) -> Result<Self, String> {
        let root = xml::parse(xml)?;
        if root.local() != "dataModel" {
            return Err(format!("not a diagram data part: {}", root.name));
        }
        Ok(Self { root })
    }

    /// A data part with nothing but the document point, laid out with
    /// `layout`, coloured with `colors` and styled with `style` (ids or the
    /// short names after the built-in prefixes).
    pub fn new(layout: &str, colors: &str, style: &str) -> Self {
        let mut model = Self {
            root: Element::new("dgm:dataModel")
                .with_attr("xmlns:dgm", DGM_NS)
                .with_attr("xmlns:a", A_NS)
                .with_attr("xmlns:r", R_NS)
                .with_child(Element::new("dgm:ptLst"))
                .with_child(Element::new("dgm:cxnLst"))
                .with_child(Element::new("dgm:bg"))
                .with_child(Element::new("dgm:whole")),
        };
        let mut doc = point(&guid(0), Some("doc"));
        doc.children.push(XmlNode::Element(
            Element::new("dgm:prSet").with_attr("phldr", "1"),
        ));
        doc.children
            .push(XmlNode::Element(Element::new("dgm:spPr")));
        doc.children.push(XmlNode::Element(empty_text()));
        model
            .root
            .child_mut("ptLst")
            .expect("made")
            .children
            .push(XmlNode::Element(doc));
        model.set_layout_id(&crate::catalog::layout_id(layout));
        model.set_colors_id(&crate::catalog::colors_id(colors));
        model.set_style_id(&crate::catalog::style_id(style));
        model
    }

    /// The part as written.
    pub fn to_xml(&self) -> String {
        self.root.to_document()
    }

    fn points(&self) -> impl Iterator<Item = &Element> {
        self.root
            .child("ptLst")
            .into_iter()
            .flat_map(|list| list.children_named("pt"))
    }

    fn connections(&self) -> impl Iterator<Item = &Element> {
        self.root
            .child("cxnLst")
            .into_iter()
            .flat_map(|list| list.children_named("cxn"))
    }

    fn document(&self) -> Option<&Element> {
        self.points().find(|pt| pt.attr("type") == Some("doc"))
    }

    fn document_set(&mut self) -> Option<&mut Element> {
        self.root
            .child_mut("ptLst")?
            .elements_mut()
            .find(|pt| pt.attr("type") == Some("doc"))
            .map(|doc| doc.child_or_insert("dgm:prSet"))
    }

    fn document_attr(&self, name: &str) -> Option<String> {
        self.document()?
            .child("prSet")?
            .attr(name)
            .map(str::to_string)
    }

    /// The layout's `uniqueId` (`loTypeId`).
    pub fn layout_id(&self) -> Option<String> {
        self.document_attr("loTypeId")
    }

    pub fn colors_id(&self) -> Option<String> {
        self.document_attr("csTypeId")
    }

    pub fn style_id(&self) -> Option<String> {
        self.document_attr("qsTypeId")
    }

    pub fn set_layout_id(&mut self, id: &str) {
        let category = crate::catalog::layout(id)
            .map(|info| info.category.id())
            .unwrap_or("list");
        if let Some(set) = self.document_set() {
            set.set_attr("loTypeId", id);
            set.set_attr("loCatId", category);
        }
    }

    pub fn set_colors_id(&mut self, id: &str) {
        let category = crate::catalog::color_scheme(id)
            .map(|scheme| scheme.category)
            .unwrap_or("accent1");
        if let Some(set) = self.document_set() {
            set.set_attr("csTypeId", id);
            set.set_attr("csCatId", category);
        }
    }

    pub fn set_style_id(&mut self, id: &str) {
        let category = crate::catalog::style(id)
            .map(|style| style.category)
            .unwrap_or("simple");
        if let Some(set) = self.document_set() {
            set.set_attr("qsTypeId", id);
            set.set_attr("qsCatId", category);
        }
    }

    /// The relationship (of the slide) the `dsp:dataModelExt` names for the
    /// cached drawing.
    pub fn drawing_rel_id(&self) -> Option<String> {
        self.root
            .descendant("dataModelExt")
            .and_then(|ext| ext.attr("relId"))
            .map(str::to_string)
    }

    /// Names `rel_id` as the cached drawing's relationship.
    pub fn set_drawing_rel_id(&mut self, rel_id: &str) {
        if let Some(ext) = find_mut(&mut self.root, "dataModelExt") {
            ext.set_attr("relId", rel_id);
            return;
        }
        let ext = Element::new("a:ext")
            .with_attr(
                "uri",
                "http://schemas.microsoft.com/office/drawing/2008/diagram",
            )
            .with_child(
                Element::new("dsp:dataModelExt")
                    .with_attr("xmlns:dsp", DSP_NS)
                    .with_attr("relId", rel_id)
                    .with_attr(
                        "minVer",
                        "http://schemas.openxmlformats.org/drawingml/2006/diagram",
                    ),
            );
        let list = self.root.child_or_insert("dgm:extLst");
        list.children.push(XmlNode::Element(ext));
    }

    /// The text pane's bullets, nested down the `parOf` connections in
    /// their order, each with the formatting its shape has of its own.
    pub fn nodes(&self) -> Vec<Node> {
        let Some(doc) = self.document().and_then(|doc| doc.attr("modelId")) else {
            return Vec::new();
        };
        let mut texts: HashMap<&str, (String, NodeKind)> = HashMap::new();
        let mut pres: HashMap<&str, &Element> = HashMap::new();
        for pt in self.points() {
            let Some(id) = pt.attr("modelId") else {
                continue;
            };
            match pt.attr("type").unwrap_or("node") {
                "node" => {
                    texts.insert(
                        id,
                        (
                            pt.child("t").map(body_text).unwrap_or_default(),
                            NodeKind::Node,
                        ),
                    );
                }
                "asst" => {
                    texts.insert(
                        id,
                        (
                            pt.child("t").map(body_text).unwrap_or_default(),
                            NodeKind::Assistant,
                        ),
                    );
                }
                "pres" => {
                    pres.insert(id, pt);
                }
                _ => {}
            }
        }
        let mut edges: Vec<(&str, &str, i64)> = Vec::new();
        let mut formats: HashMap<&str, ShapeFormat> = HashMap::new();
        for cxn in self.connections() {
            let (Some(source), Some(destination)) = (cxn.attr("srcId"), cxn.attr("destId")) else {
                continue;
            };
            match cxn.attr("type").unwrap_or("parOf") {
                "parOf" => edges.push((
                    source,
                    destination,
                    cxn.attr("srcOrd").and_then(|o| o.parse().ok()).unwrap_or(0),
                )),
                "presOf" if cxn.attr("destOrd").unwrap_or("0") == "0" => {
                    if let Some(pt) = pres.get(destination) {
                        let format = read_format(pt);
                        if !format.is_empty() {
                            formats.entry(source).or_insert(format);
                        }
                    }
                }
                _ => {}
            }
        }
        edges.sort_by_key(|(_, _, order)| *order);
        fn build(
            id: &str,
            edges: &[(&str, &str, i64)],
            texts: &HashMap<&str, (String, NodeKind)>,
            formats: &HashMap<&str, ShapeFormat>,
            seen: &mut HashSet<String>,
        ) -> Vec<Node> {
            let mut out = Vec::new();
            for (source, destination, _) in edges {
                if *source != id || !texts.contains_key(destination) {
                    continue;
                }
                if !seen.insert(destination.to_string()) {
                    continue;
                }
                let (text, kind) = &texts[destination];
                out.push(Node {
                    id: destination.to_string(),
                    kind: *kind,
                    text: text.clone(),
                    format: formats.get(destination).cloned().unwrap_or_default(),
                    children: build(destination, edges, texts, formats, seen),
                });
            }
            out
        }
        build(doc, &edges, &texts, &formats, &mut HashSet::new())
    }

    fn tree_shape(&self) -> Shape {
        fn walk(nodes: &[Node], parent: &str, out: &mut Shape) {
            for node in nodes {
                out.push((node.id.clone(), parent.to_string(), node.kind));
                walk(&node.children, &node.id, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.nodes(), "", &mut out);
        out
    }

    fn next_seed(&self) -> u64 {
        let mut seed = 1 + self.points().count() as u64 + self.connections().count() as u64;
        let ids: HashSet<String> = self
            .points()
            .chain(self.connections())
            .filter_map(|el| el.attr("modelId").map(str::to_string))
            .collect();
        while ids.contains(&guid(seed)) {
            seed += 1;
        }
        seed
    }

    /// Makes the bullets `nodes`: points with an id keep their markup and
    /// get the new text and place; points without one are added; points
    /// no longer there go, with their connections. When the nesting
    /// changed, the presentation points PowerPoint derived from the old
    /// nesting go too (it derives them again), all but those carrying a
    /// shape's own formatting. Returns the nodes with their ids filled in.
    pub fn set_nodes(&mut self, nodes: &[Node]) -> Vec<Node> {
        let mut seed = self.next_seed();
        let mut fresh = || {
            let id = guid(seed);
            seed += 1;
            id
        };
        // Ids for new bullets, unique ones for duplicated ids.
        let mut used = HashSet::new();
        fn assign(
            nodes: &mut [Node],
            used: &mut HashSet<String>,
            fresh: &mut dyn FnMut() -> String,
        ) {
            for node in nodes {
                if node.id.is_empty() || !used.insert(node.id.clone()) {
                    node.id = fresh();
                    used.insert(node.id.clone());
                }
                assign(&mut node.children, used, fresh);
            }
        }
        let mut nodes = nodes.to_vec();
        if let Some(doc) = self.document().and_then(|doc| doc.attr("modelId")) {
            used.insert(doc.to_string());
        }
        assign(&mut nodes, &mut used, &mut fresh);

        let before = self.tree_shape();
        let doc_id = self
            .document()
            .and_then(|doc| doc.attr("modelId"))
            .unwrap_or_default()
            .to_string();
        let mut flat: Vec<(&Node, String, usize)> = Vec::new();
        fn flatten<'a>(nodes: &'a [Node], parent: &str, out: &mut Vec<(&'a Node, String, usize)>) {
            for (order, node) in nodes.iter().enumerate() {
                out.push((node, parent.to_string(), order));
                flatten(&node.children, &node.id, out);
            }
        }
        flatten(&nodes, &doc_id, &mut flat);
        let after: Shape = {
            fn walk(nodes: &[Node], parent: &str, out: &mut Shape) {
                for node in nodes {
                    out.push((node.id.clone(), parent.to_string(), node.kind));
                    walk(&node.children, &node.id, out);
                }
            }
            let mut out = Vec::new();
            walk(&nodes, "", &mut out);
            out
        };
        let restructured = before != after;

        let points: Vec<Element> = self.points().cloned().collect();
        let connections: Vec<Element> = self.connections().cloned().collect();
        let by_id: HashMap<String, &Element> = points
            .iter()
            .filter_map(|pt| pt.attr("modelId").map(|id| (id.to_string(), pt)))
            .collect();
        let par_of: HashMap<String, &Element> = connections
            .iter()
            .filter(|cxn| matches!(cxn.attr("type"), None | Some("parOf")))
            .filter_map(|cxn| cxn.attr("destId").map(|id| (id.to_string(), cxn)))
            .collect();
        let alive: HashSet<&str> = flat.iter().map(|(node, ..)| node.id.as_str()).collect();

        let mut new_points = Vec::new();
        let mut new_connections = Vec::new();
        let mut transitions = HashSet::new();
        for pt in &points {
            if pt.attr("type") == Some("doc") {
                new_points.push(pt.clone());
            }
        }
        for (node, parent, order) in &flat {
            let mut pt = by_id
                .get(&node.id)
                .map(|pt| (*pt).clone())
                .unwrap_or_else(|| {
                    let mut pt = point(&node.id, None);
                    pt.children.push(XmlNode::Element(
                        Element::new("dgm:prSet").with_attr("phldrT", "[テキスト]"),
                    ));
                    pt.children.push(XmlNode::Element(Element::new("dgm:spPr")));
                    pt.children.push(XmlNode::Element(empty_text()));
                    pt
                });
            match node.kind {
                NodeKind::Node => pt.remove_attr("type"),
                NodeKind::Assistant => pt.set_attr("type", "asst"),
            }
            let current = pt.child("t").map(body_text).unwrap_or_default();
            if current != node.text || pt.child("t").is_none() {
                let body = pt.child_or_insert("dgm:t");
                set_body_text(body, &node.text);
                if let Some(set) = pt.child_mut("prSet") {
                    // Typed text is no longer the placeholder's.
                    set.remove_attr("phldr");
                }
            }
            new_points.push(pt);
            let mut cxn = par_of
                .get(&node.id)
                .map(|cxn| (*cxn).clone())
                .unwrap_or_else(|| {
                    let id = fresh();
                    let par = fresh();
                    let sib = fresh();
                    Element::new("dgm:cxn")
                        .with_attr("modelId", id)
                        .with_attr("srcId", "")
                        .with_attr("destId", node.id.clone())
                        .with_attr("srcOrd", "0")
                        .with_attr("destOrd", "0")
                        .with_attr("parTransId", par)
                        .with_attr("sibTransId", sib)
                });
            cxn.set_attr("srcId", parent.clone());
            cxn.set_attr("srcOrd", order.to_string());
            let connection_id = cxn.attr("modelId").unwrap_or_default().to_string();
            for (attr, kind) in [("parTransId", "parTrans"), ("sibTransId", "sibTrans")] {
                let Some(id) = cxn.attr(attr).map(str::to_string) else {
                    continue;
                };
                transitions.insert(id.clone());
                let transition = by_id.get(&id).map(|pt| (*pt).clone()).unwrap_or_else(|| {
                    let mut pt = point(&id, Some(kind));
                    pt.set_attr("cxnId", connection_id.clone());
                    pt.children
                        .push(XmlNode::Element(Element::new("dgm:prSet")));
                    pt.children.push(XmlNode::Element(Element::new("dgm:spPr")));
                    pt.children.push(XmlNode::Element(empty_text()));
                    pt
                });
                new_points.push(transition);
            }
            new_connections.push(cxn);
        }

        // Presentation points: all of them when only the text changed;
        // otherwise those that carry a surviving shape's own formatting.
        let pres_kept: HashSet<String> = points
            .iter()
            .filter(|pt| pt.attr("type") == Some("pres"))
            .filter(|pt| {
                let assoc = pt
                    .child("prSet")
                    .and_then(|set| set.attr("presAssocID"))
                    .unwrap_or_default();
                let alive = alive.contains(assoc) || assoc == doc_id || transitions.contains(assoc);
                alive && (!restructured || !read_format(pt).is_empty())
            })
            .filter_map(|pt| pt.attr("modelId").map(str::to_string))
            .collect();
        for pt in &points {
            if pt.attr("modelId").is_some_and(|id| pres_kept.contains(id)) {
                new_points.push(pt.clone());
            }
        }
        for cxn in &connections {
            match cxn.attr("type") {
                Some("presOf") => {
                    let keep = cxn.attr("destId").is_some_and(|id| pres_kept.contains(id))
                        && cxn.attr("srcId").is_some_and(|id| {
                            alive.contains(id) || id == doc_id || transitions.contains(id)
                        });
                    if keep {
                        new_connections.push(cxn.clone());
                    }
                }
                Some("presParOf") if !restructured => {
                    let keep = cxn.attr("srcId").is_some_and(|id| pres_kept.contains(id))
                        && cxn.attr("destId").is_some_and(|id| pres_kept.contains(id));
                    if keep {
                        new_connections.push(cxn.clone());
                    }
                }
                _ => {}
            }
        }
        if let Some(list) = self.root.child_mut("ptLst") {
            list.children = new_points.into_iter().map(XmlNode::Element).collect();
        }
        if let Some(list) = self.root.child_mut("cxnLst") {
            list.children = new_connections.into_iter().map(XmlNode::Element).collect();
        }
        for (node, ..) in &flat {
            self.set_format(&node.id, &node.format);
        }
        nodes
    }

    /// Gives the bullet `id`'s shape `format` of its own: on the
    /// presentation point that shows it, made (named `pres_name`, the
    /// layout's name for a bullet's shape) when there is none.
    pub fn set_format(&mut self, id: &str, format: &ShapeFormat) {
        let current = self
            .nodes_flat()
            .into_iter()
            .find(|node| node.id == id)
            .map(|node| node.format)
            .unwrap_or_default();
        if &current == format {
            return;
        }
        let pres_id = self
            .connections()
            .find(|cxn| {
                cxn.attr("type") == Some("presOf")
                    && cxn.attr("srcId") == Some(id)
                    && cxn.attr("destOrd").unwrap_or("0") == "0"
            })
            .and_then(|cxn| cxn.attr("destId"))
            .map(str::to_string)
            .filter(|pres| {
                self.points()
                    .any(|pt| pt.attr("modelId") == Some(pres.as_str()))
            });
        let pres_id = match pres_id {
            Some(pres) => pres,
            None => {
                if format.is_empty() {
                    return;
                }
                let mut seed = self.next_seed();
                let pres = guid(seed);
                seed += 1;
                let cxn_id = guid(seed);
                let layout = self.layout_id().unwrap_or_default();
                let name = crate::catalog::layout(&layout)
                    .map(|info| info.node_name)
                    .unwrap_or("node");
                let mut pt = point(&pres, Some("pres"));
                pt.children.push(XmlNode::Element(
                    Element::new("dgm:prSet")
                        .with_attr("presAssocID", id)
                        .with_attr("presName", name)
                        .with_attr("presStyleLbl", "node1"),
                ));
                pt.children.push(XmlNode::Element(Element::new("dgm:spPr")));
                if let Some(list) = self.root.child_mut("ptLst") {
                    list.children.push(XmlNode::Element(pt));
                }
                let cxn = Element::new("dgm:cxn")
                    .with_attr("modelId", cxn_id)
                    .with_attr("type", "presOf")
                    .with_attr("srcId", id)
                    .with_attr("destId", pres.clone())
                    .with_attr("srcOrd", "0")
                    .with_attr("destOrd", "0")
                    .with_attr("presId", layout);
                if let Some(list) = self.root.child_mut("cxnLst") {
                    list.children.push(XmlNode::Element(cxn));
                }
                pres
            }
        };
        if let Some(pt) = self.root.child_mut("ptLst").and_then(|list| {
            list.elements_mut()
                .find(|pt| pt.attr("modelId") == Some(pres_id.as_str()))
        }) {
            let sp_pr = pt.child_or_insert("dgm:spPr");
            write_format(sp_pr, format);
        }
    }

    /// For each presentation point (a shape of the drawing PowerPoint
    /// caches: its `modelId`), the bullets whose text it shows, in order.
    pub fn shown_in(&self) -> HashMap<String, Vec<String>> {
        let mut out: HashMap<String, Vec<(i64, String)>> = HashMap::new();
        for cxn in self.connections() {
            if cxn.attr("type") != Some("presOf") {
                continue;
            }
            let (Some(source), Some(destination)) = (cxn.attr("srcId"), cxn.attr("destId")) else {
                continue;
            };
            let order = cxn.attr("srcOrd").and_then(|o| o.parse().ok()).unwrap_or(0);
            out.entry(destination.to_string())
                .or_default()
                .push((order, source.to_string()));
        }
        out.into_iter()
            .map(|(pres, mut nodes)| {
                nodes.sort_by_key(|(order, _)| *order);
                (pres, nodes.into_iter().map(|(_, id)| id).collect())
            })
            .collect()
    }

    /// Every bullet, depth first.
    pub fn nodes_flat(&self) -> Vec<Node> {
        outline(&self.nodes())
            .into_iter()
            .map(|(_, node)| {
                let mut node = node.clone();
                node.children.clear();
                node
            })
            .collect()
    }
}

fn find_mut<'a>(element: &'a mut Element, name: &str) -> Option<&'a mut Element> {
    for child in element.elements_mut() {
        if child.local() == name {
            return Some(child);
        }
        if let Some(found) = find_mut(child, name) {
            return Some(found);
        }
    }
    None
}
