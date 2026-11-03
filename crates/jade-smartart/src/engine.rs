//! Lays a diagram's bullets out as PowerPoint's built-in layouts do.
//!
//! Each layout is a function from the bullets and the frame to shapes, its
//! proportions measured from what PowerPoint draws for the same data: the
//! box shapes and their spacing as fractions of each other and of the
//! frame, the arrows and connectors between them, which text sits in which
//! shape. Text is then sized as PowerPoint sizes it: the largest size (up
//! to the layout's limit) at which every shape of a kind holds its text,
//! the same for all of them.

use crate::catalog;
use crate::data::{Node, NodeKind, ShapeFormat};
use crate::layout_def::Algorithm;

/// One paragraph of a shape's text.
#[derive(Clone, Debug, PartialEq)]
pub struct Para {
    pub text: String,
    /// 0 for a bullet's own text, 1… for the bullets under it.
    pub level: usize,
}

/// How a shape's paint is chosen.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    /// From the colours and style of `label`, as shape `index` of `count`.
    Label {
        label: &'static str,
        index: usize,
        count: usize,
    },
    /// No fill and no line: text alone.
    TextOnly,
    /// Text alone, coloured as `label`'s text: over a shape of its own
    /// drawn turned over.
    TextOf {
        label: &'static str,
        index: usize,
        count: usize,
    },
}

/// A shape of the drawing, in EMU within the frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawn {
    /// The bullet this shape shows (its `modelId`), for formatting and
    /// selection.
    pub node: Option<String>,
    pub preset: String,
    pub adjust: Vec<(String, i64)>,
    /// A polyline (in the frame's EMU) drawn instead of a preset: the
    /// connectors of hierarchies.
    pub path: Option<Vec<(f64, f64)>>,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Clockwise degrees.
    pub rot: f64,
    pub paint: Paint,
    pub paras: Vec<Para>,
    /// Where the text goes when not the whole shape (in the frame).
    pub text_box: Option<[f64; 4]>,
    /// Text at the top left rather than the middle.
    pub top_left: bool,
    /// Bullets before every paragraph, not only the sub-points.
    pub bullets: bool,
    /// Text inset as a fraction of the font size.
    pub inset: f64,
    /// Shapes of one group get one font size.
    pub group: usize,
    pub max_pt: f64,
    /// The font size, filled in once every shape is placed.
    pub size_pt: f64,
    /// A picture placeholder.
    pub picture: bool,
    pub format: ShapeFormat,
}

impl Drawn {
    fn new(preset: &str, x: f64, y: f64, w: f64, h: f64, paint: Paint) -> Self {
        Self {
            node: None,
            preset: preset.to_string(),
            adjust: Vec::new(),
            path: None,
            x,
            y,
            w,
            h,
            rot: 0.0,
            paint,
            paras: Vec::new(),
            text_box: None,
            top_left: false,
            bullets: false,
            inset: 0.3,
            group: 0,
            max_pt: 65.0,
            size_pt: 0.0,
            picture: false,
            format: ShapeFormat::default(),
        }
    }

    fn adj(mut self, name: &str, value: i64) -> Self {
        self.adjust.push((name.to_string(), value));
        self
    }

    fn rot(mut self, degrees: f64) -> Self {
        self.rot = degrees;
        self
    }

    fn group(mut self, group: usize) -> Self {
        self.group = group;
        self
    }

    fn text_box(mut self, rect: [f64; 4]) -> Self {
        self.text_box = Some(rect);
        self
    }

    /// Shows `node`: its text, and with `children` the bullets under it.
    fn show(mut self, node: &Node, children: bool) -> Self {
        self.node = Some(node.id.clone());
        self.format = node.format.clone();
        self.paras = own_text(node);
        if children {
            push_children(&mut self.paras, &node.children, 1);
            if !node.children.is_empty() {
                self.top_left = true;
            }
        }
        self
    }

    /// Shows the bullets under `node` alone, as a list.
    fn show_children(mut self, node: &Node) -> Self {
        push_children(&mut self.paras, &node.children, 0);
        self.top_left = true;
        self.bullets = true;
        self
    }

    fn picture(mut self, node: &Node) -> Self {
        self.picture = true;
        self.node = Some(node.id.clone());
        self.format.picture = node.format.picture.clone();
        self
    }
}

fn own_text(node: &Node) -> Vec<Para> {
    node.text
        .split('\n')
        .map(|line| Para {
            text: line.to_string(),
            level: 0,
        })
        .collect()
}

fn push_children(out: &mut Vec<Para>, nodes: &[Node], level: usize) {
    for node in nodes {
        for line in node.text.split('\n') {
            out.push(Para {
                text: line.to_string(),
                level,
            });
        }
        push_children(out, &node.children, level + 1);
    }
}

fn label(label: &'static str, index: usize, count: usize) -> Paint {
    Paint::Label {
        label,
        index,
        count,
    }
}

/// Lays `nodes` out with the layout `layout_id` (a built-in id, or any id
/// with `algorithm` read from its definition) in a `width` × `height` EMU
/// frame.
pub fn lay_out(
    layout_id: &str,
    algorithm: Option<Algorithm>,
    nodes: &[Node],
    width: f64,
    height: f64,
) -> Vec<Drawn> {
    let short = catalog::layout(layout_id)
        .map(|info| info.short)
        .unwrap_or_else(
            || match algorithm.unwrap_or_else(|| Algorithm::guess(layout_id)) {
                Algorithm::Snake => "default",
                Algorithm::Linear => "process1",
                Algorithm::Cycle => "cycle2",
                Algorithm::Hierarchy => "orgChart1",
                Algorithm::Pyramid => "pyramid1",
                Algorithm::Venn => "venn1",
                Algorithm::Matrix => "matrix3",
                Algorithm::Picture => "pList1",
            },
        );
    let (w, h) = (width.max(1.0), height.max(1.0));
    let mut out = match short {
        "default" => block_list(nodes, w, h),
        "vList2" => vertical_bullets(nodes, w, h),
        "hList1" => horizontal_bullets(nodes, w, h),
        "lProcess2" => grouped_list(nodes, w, h),
        "vList5" => vertical_blocks(nodes, w, h),
        "process1" => process(nodes, w, h),
        "process2" => vertical_process(nodes, w, h),
        "chevron1" => chevrons(nodes, w, h),
        "hProcess9" => continuous_blocks(nodes, w, h),
        "bProcess4" => bending(nodes, w, h),
        "cycle2" => cycle(nodes, w, h),
        "cycle1" => text_cycle(nodes, w, h),
        "cycle3" => block_cycle(nodes, w, h),
        "radial1" => radial(nodes, w, h),
        "chart3" => pie(nodes, w, h),
        "orgChart1" => org_chart(nodes, w, h, false),
        s if s.ends_with("NameandTitleOrganizationalChart") => org_chart(nodes, w, h, true),
        "hierarchy1" => hierarchy(nodes, w, h),
        "hierarchy2" => horizontal_hierarchy(nodes, w, h),
        "hierarchy3" => hierarchy_list(nodes, w, h),
        "venn1" => venn(nodes, w, h),
        "venn3" => linear_venn(nodes, w, h),
        "venn2" => stacked_venn(nodes, w, h),
        "target1" => nested_target(nodes, w, h),
        "equation1" => equation(nodes, w, h),
        "matrix3" => matrix(nodes, w, h, false),
        "matrix2" => matrix(nodes, w, h, true),
        "matrix1" => titled_matrix(nodes, w, h),
        "pyramid1" => pyramid(nodes, w, h, false),
        "pyramid3" => pyramid(nodes, w, h, true),
        "pyramid2" => pyramid_list(nodes, w, h),
        "pyramid4" => segmented_pyramid(nodes, w, h),
        "pList1" => picture_captions(nodes, w, h),
        s if s.ends_with("TitledPictureBlocks") => titled_pictures(nodes, w, h),
        s if s.ends_with("PictureStrips") => picture_strips(nodes, w, h),
        s if s.ends_with("CaptionedPictures") => captioned_pictures(nodes, w, h),
        _ => block_list(nodes, w, h),
    };
    fit_text(&mut out);
    out
}

/// Columns and rows for `count` boxes of aspect `ratio` (height / width)
/// spaced `gap` box-widths apart, as big as the frame allows: returns
/// (columns, box width).
fn grid(count: usize, w: f64, h: f64, ratio: f64, gap_x: f64, gap_y: f64) -> (usize, f64) {
    let mut best = (1, 0.0);
    for columns in 1..=count.max(1) {
        let rows = count.max(1).div_ceil(columns);
        let by_width = w / (columns as f64 + (columns - 1) as f64 * gap_x);
        let by_height = h / (rows as f64 * ratio + (rows - 1) as f64 * gap_y);
        let size = by_width.min(by_height);
        if size > best.1 + 1.0 {
            best = (columns, size);
        }
    }
    best
}

/// Places `count` cells of `cw` × `ch` in a grid of `columns`, the last row
/// centred, the whole centred in the frame: each cell's top left.
fn grid_cells(
    count: usize,
    columns: usize,
    (cw, ch): (f64, f64),
    (gx, gy): (f64, f64),
    w: f64,
    h: f64,
) -> Vec<(f64, f64)> {
    let rows = count.max(1).div_ceil(columns);
    let used_w = columns as f64 * cw + (columns - 1) as f64 * gx;
    let used_h = rows as f64 * ch + (rows - 1) as f64 * gy;
    let (left, top) = ((w - used_w) / 2.0, (h - used_h) / 2.0);
    (0..count)
        .map(|index| {
            let (row, column) = (index / columns, index % columns);
            let in_row = if row == rows - 1 {
                count - row * columns
            } else {
                columns
            };
            let row_left = left + (used_w - (in_row as f64 * cw + (in_row - 1) as f64 * gx)) / 2.0;
            (
                row_left + column as f64 * (cw + gx),
                top + row as f64 * (ch + gy),
            )
        })
        .collect()
}

fn block_list(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len();
    let (columns, bw) = grid(n, w, h, 0.6, 0.1, 0.1 / 0.6 * 0.6);
    let bh = bw * 0.6;
    let gap = bw * 0.1;
    grid_cells(n, columns, (bw, bh), (gap, gap), w, h)
        .into_iter()
        .zip(nodes)
        .enumerate()
        .map(|(i, ((x, y), node))| {
            Drawn::new("rect", x, y, bw, bh, label("node1", i, n)).show(node, true)
        })
        .collect()
}

fn vertical_bullets(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len();
    let lines = |node: &Node| {
        let mut paras = Vec::new();
        push_children(&mut paras, &node.children, 0);
        paras.len()
    };
    let mut units = 0.0;
    for (i, node) in nodes.iter().enumerate() {
        units += 1.0;
        let k = lines(node);
        if k > 0 {
            units += 0.4315 * k as f64 + 0.259;
        } else if i + 1 < n {
            units += 0.12;
        }
    }
    let p = (h / units.max(1.0)).min(w * 0.4);
    let mut y = (h - p * units).max(0.0) / 2.0;
    let mut out = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        out.push(
            Drawn::new("roundRect", 0.0, y, w, p, label("node1", i, n))
                .show(node, false)
                .group(0),
        );
        y += p;
        let k = lines(node);
        if k > 0 {
            let bh = p * (0.4315 * k as f64 + 0.259);
            out.push(
                Drawn::new("rect", 0.0, y, w, bh, Paint::TextOnly)
                    .show_children(node)
                    .group(1),
            );
            y += bh;
        } else {
            y += p * 0.12;
        }
    }
    out
}

fn horizontal_bullets(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let cw = (w / (n as f64 + 0.14 * (n - 1) as f64)).min(w);
    let (head, body) = if cw * 1.18 > h {
        (h * 0.234, h * 0.758)
    } else {
        (cw * 0.4, cw * 0.78)
    };
    let pitch = if n > 1 {
        (w - cw) / (n - 1) as f64
    } else {
        0.0
    };
    let top = (h - head - body) / 2.0;
    let mut out = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let x = i as f64 * pitch;
        out.push(
            Drawn::new("rect", x, top, cw, head, label("alignNode1", i, n))
                .show(node, false)
                .group(0),
        );
        let mut body_shape = Drawn::new(
            "rect",
            x,
            top + head,
            cw,
            body,
            label("alignAccFollowNode1", i, n),
        )
        .show_children(node)
        .group(0);
        body_shape.node = Some(node.id.clone());
        out.push(body_shape);
    }
    out
}

fn grouped_list(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let cw = w / (n as f64 + 0.075 * (n - 1) as f64);
    let gap = cw * 0.075;
    let mut out = Vec::new();
    let all: usize = nodes.iter().map(|node| node.children.len()).sum();
    let mut child_index = 0;
    for (i, node) in nodes.iter().enumerate() {
        let x = i as f64 * (cw + gap);
        out.push(
            Drawn::new("roundRect", x, 0.0, cw, h, label("bgShp", i, n))
                .adj("adj", 10_000)
                .show(node, false)
                .text_box([x, 0.0, cw, h * 0.3])
                .group(0),
        );
        let k = node.children.len();
        if k > 0 {
            let region = h * 0.65;
            let ch = region / (k as f64 + 0.154 * (k - 1) as f64);
            for (j, child) in node.children.iter().enumerate() {
                out.push(
                    Drawn::new(
                        "roundRect",
                        x + cw * 0.1,
                        h * 0.3 + j as f64 * ch * 1.154,
                        cw * 0.8,
                        ch,
                        label("node1", child_index, all),
                    )
                    .adj("adj", 10_000)
                    .show(child, true)
                    .group(1),
                );
                child_index += 1;
            }
        }
    }
    out
}

fn vertical_blocks(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let ph = h / (n as f64 + 0.05 * (n - 1) as f64);
    let any_children = nodes.iter().any(|node| !node.children.is_empty());
    let pw = (w * 0.36).min(ph * 3.0).max(w * 0.2);
    let left = if any_children { 0.0 } else { (w - pw) / 2.0 };
    let mut out = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let y = i as f64 * ph * 1.05;
        if !node.children.is_empty() {
            let (bw, bh) = (w - pw, ph * 0.8);
            // A tab turned on its side: drawn as round2SameRect turned 90°.
            let (cx, cy) = (pw + bw / 2.0, y + ph / 2.0);
            out.push(
                Drawn::new(
                    "round2SameRect",
                    cx - bh / 2.0,
                    cy - bw / 2.0,
                    bh,
                    bw,
                    label("alignAccFollowNode1", i, n),
                )
                .rot(90.0)
                .show_children(node)
                .text_box([pw, cy - bh / 2.0, bw, bh])
                .group(1),
            );
            if let Some(last) = out.last_mut() {
                last.node = Some(node.id.clone());
            }
        }
        out.push(
            Drawn::new("roundRect", left, y, pw, ph, label("node1", i, n))
                .show(node, false)
                .group(0),
        );
    }
    out
}

fn process(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let bw = (w / (n as f64 + 0.4 * (n - 1) as f64)).min(h / 0.6);
    let bh = bw * 0.6;
    let total = bw * (n as f64 + 0.4 * (n - 1) as f64);
    let left = (w - total) / 2.0;
    let top = (h - bh) / 2.0;
    let mut out = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let x = left + i as f64 * bw * 1.4;
        out.push(
            Drawn::new("roundRect", x, top, bw, bh, label("node1", i, n))
                .adj("adj", 10_000)
                .show(node, true),
        );
        if i + 1 < nodes.len() {
            let (aw, ah) = (bw * 0.212, bw * 0.248);
            let mut arrow = Drawn::new(
                "rightArrow",
                x + bw * 1.1,
                top + (bh - ah) / 2.0,
                aw,
                ah,
                label("sibTrans2D1", i, n - 1),
            )
            .adj("adj1", 60_000)
            .adj("adj2", 50_000)
            .group(9);
            arrow.node = Some(node.id.clone());
            out.push(arrow);
        }
    }
    out
}

fn vertical_process(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let bh = h / (n as f64 + 0.5 * (n - 1) as f64);
    let bw = (bh * 1.8).min(w);
    let total = bh * (n as f64 + 0.5 * (n - 1) as f64);
    let top = (h - total) / 2.0;
    let left = (w - bw) / 2.0;
    let mut out = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let y = top + i as f64 * bh * 1.5;
        out.push(
            Drawn::new("roundRect", left, y, bw, bh, label("node1", i, n))
                .adj("adj", 10_000)
                .show(node, true),
        );
        if i + 1 < nodes.len() {
            let (along, across) = (bw * 0.2083, bw * 0.25);
            let (cx, cy) = (w / 2.0, y + bh * 1.25);
            let mut arrow = Drawn::new(
                "rightArrow",
                cx - along / 2.0,
                cy - across / 2.0,
                along,
                across,
                label("sibTrans2D1", i, n - 1),
            )
            .adj("adj1", 60_000)
            .adj("adj2", 50_000)
            .rot(90.0)
            .group(9);
            arrow.node = Some(node.id.clone());
            out.push(arrow);
        }
    }
    out
}

fn chevrons(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let any_children = nodes.iter().any(|node| !node.children.is_empty());
    let cw = w / (n as f64 * 0.9 + 0.1);
    let mut ch = cw * 0.4;
    let mut child_h = if any_children { cw * 0.693 } else { 0.0 };
    let gap = if any_children { ch * 0.125 } else { 0.0 };
    let mut gap = gap;
    if ch + gap + child_h > h {
        // Too tall: the bullets under a chevron take one and a half times
        // its height.
        ch = h * 0.994 / if any_children { 2.625 } else { 1.0 };
        gap = if any_children { ch * 0.125 } else { 0.0 };
        child_h = if any_children { ch * 1.5 } else { 0.0 };
    }
    let top = (h - (ch + gap + child_h)) / 2.0;
    let mut out = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let x = i as f64 * cw * 0.9;
        out.push(
            Drawn::new("chevron", x, top, cw, ch, label("node1", i, n))
                .show(node, false)
                .group(0),
        );
        if !node.children.is_empty() {
            let mut list = Drawn::new(
                "rect",
                x,
                top + ch + gap,
                cw * 0.8,
                child_h,
                Paint::TextOnly,
            )
            .show_children(node)
            .group(1);
            list.node = Some(node.id.clone());
            out.push(list);
        }
    }
    out
}

fn continuous_blocks(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let mut out = vec![Drawn::new(
        "rightArrow",
        w * 0.075,
        0.0,
        w * 0.85,
        h,
        label("bgShp", 0, 1),
    )
    .group(9)];
    let bw = (w / (n as f64 + 0.1455 * (n - 1) as f64)).min(w * 0.3);
    let pitch = if n > 1 {
        (w - bw) / (n - 1) as f64
    } else {
        0.0
    };
    let left = if n > 1 { 0.0 } else { (w - bw) / 2.0 };
    for (i, node) in nodes.iter().enumerate() {
        out.push(
            Drawn::new(
                "roundRect",
                left + i as f64 * pitch,
                h * 0.3,
                bw,
                h * 0.4,
                label("node1", i, n),
            )
            .show(node, true),
        );
    }
    out
}

fn bending(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    // Column by column, down then up: the best grid for boxes 0.6 high,
    // 0.33 apart across and 0.15 apart down.
    let mut best = (1usize, 1usize, 0.0f64);
    for columns in 1..=n {
        let rows = n.div_ceil(columns);
        let size = (w / (columns as f64 + 0.33 * (columns - 1) as f64))
            .min(h / (0.6 * rows as f64 + 0.15 * (rows - 1) as f64));
        if size > best.2 + 1.0 {
            best = (columns, rows, size);
        }
    }
    let (columns, rows, bw) = best;
    let bh = bw * 0.6;
    let (gx, gy) = (bw * 0.33, bw * 0.15);
    let used_w = columns as f64 * bw + (columns - 1) as f64 * gx;
    let used_h = rows as f64 * bh + (rows - 1) as f64 * gy;
    let (left, top) = ((w - used_w) / 2.0, (h - used_h) / 2.0);
    let cell = |i: usize| {
        let column = i / rows;
        let mut row = i % rows;
        if column % 2 == 1 {
            row = rows - 1 - row;
        }
        (
            left + column as f64 * (bw + gx),
            top + row as f64 * (bh + gy),
        )
    };
    let mut out = Vec::new();
    // Connectors first, behind the boxes.
    let t = bw * 0.09;
    for (i, node) in nodes.iter().enumerate().take(n.saturating_sub(1)) {
        let (a, b) = (cell(i), cell(i + 1));
        let (ax, ay) = (a.0 + bw / 2.0, a.1 + bh / 2.0);
        let (bx, by) = (b.0 + bw / 2.0, b.1 + bh / 2.0);
        let mut bar = if (ax - bx).abs() < 1.0 {
            Drawn::new(
                "rect",
                ax - t / 2.0,
                ay.min(by),
                t,
                (ay - by).abs(),
                label("bgSibTrans2D1", i, n - 1),
            )
        } else {
            Drawn::new(
                "rect",
                ax.min(bx),
                ay - t / 2.0,
                (ax - bx).abs(),
                t,
                label("bgSibTrans2D1", i, n - 1),
            )
        };
        bar.node = Some(node.id.clone());
        out.push(bar.group(9));
    }
    for (i, node) in nodes.iter().enumerate() {
        let (x, y) = cell(i);
        out.push(
            Drawn::new("roundRect", x, y, bw, bh, label("node1", i, n))
                .adj("adj", 10_000)
                .show(node, true),
        );
    }
    out
}

/// Bounding box of shapes (their unrotated boxes).
fn bounds(shapes: &[Drawn]) -> Option<(f64, f64, f64, f64)> {
    let mut b: Option<(f64, f64, f64, f64)> = None;
    for s in shapes {
        let (x0, y0, x1, y1) = if (s.rot - 90.0).abs() < 0.1 || (s.rot - 270.0).abs() < 0.1 {
            let (cx, cy) = (s.x + s.w / 2.0, s.y + s.h / 2.0);
            (
                cx - s.h / 2.0,
                cy - s.w / 2.0,
                cx + s.h / 2.0,
                cy + s.w / 2.0,
            )
        } else {
            (s.x, s.y, s.x + s.w, s.y + s.h)
        };
        b = Some(match b {
            None => (x0, y0, x1, y1),
            Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
        });
    }
    b
}

/// Scales (uniformly) and moves shapes laid out in any units so their
/// bounds are centred in the frame as large as it allows.
fn fit_into(mut shapes: Vec<Drawn>, w: f64, h: f64) -> Vec<Drawn> {
    let Some((x0, y0, x1, y1)) = bounds(&shapes) else {
        return shapes;
    };
    let (bw, bh) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
    let k = (w / bw).min(h / bh);
    let (dx, dy) = ((w - bw * k) / 2.0, (h - bh * k) / 2.0);
    for s in &mut shapes {
        s.x = (s.x - x0) * k + dx;
        s.y = (s.y - y0) * k + dy;
        s.w *= k;
        s.h *= k;
        if let Some(path) = &mut s.path {
            for p in path.iter_mut() {
                p.0 = (p.0 - x0) * k + dx;
                p.1 = (p.1 - y0) * k + dy;
            }
        }
        if let Some(tb) = &mut s.text_box {
            tb[0] = (tb[0] - x0) * k + dx;
            tb[1] = (tb[1] - y0) * k + dy;
            tb[2] *= k;
            tb[3] *= k;
        }
    }
    shapes
}

fn ring(n: usize, index: usize) -> f64 {
    -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * index as f64 / n.max(1) as f64
}

fn cycle(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    // The node's share of the ring, as PowerPoint sizes it.
    let d = match n {
        1 => 1.0,
        2 => 0.5,
        3 => 0.4344,
        4 => 0.36,
        5 => 0.3018,
        6 => 0.26,
        7 => 0.23,
        _ => (1.6 / n as f64).min(0.2),
    };
    let size = 1.0;
    let r = (size - d) / 2.0;
    let mut out = Vec::new();
    let center = |i: usize| (r * ring(n, i).cos(), r * ring(n, i).sin());
    for (i, node) in nodes.iter().enumerate() {
        let (cx, cy) = center(i);
        out.push(
            Drawn::new(
                "ellipse",
                cx - d / 2.0,
                cy - d / 2.0,
                d,
                d,
                label("node1", i, n),
            )
            .show(node, true),
        );
        if n > 1 {
            let (nx, ny) = center((i + 1) % n);
            let (mx, my) = ((cx + nx) / 2.0, (cy + ny) / 2.0);
            let angle = (ny - cy).atan2(nx - cx).to_degrees().rem_euclid(360.0);
            let (aw, ah) = (d * 0.2667, d * 0.3375);
            let mut arrow = Drawn::new(
                "rightArrow",
                mx - aw / 2.0,
                my - ah / 2.0,
                aw,
                ah,
                label("sibTrans2D1", i, n),
            )
            .adj("adj1", 60_000)
            .adj("adj2", 50_000)
            .rot(angle)
            .group(9);
            arrow.node = Some(node.id.clone());
            out.push(arrow);
        }
    }
    fit_into(out, w, h)
}

fn text_cycle(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let d_ring = 1.0;
    let r = d_ring / 2.0;
    let t = (d_ring * (std::f64::consts::PI / n as f64).sin() * 0.45).min(0.42);
    let mut out = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let a = ring(n, i);
        let (cx, cy) = (r * a.cos(), r * a.sin());
        out.push(
            Drawn::new("rect", cx - t / 2.0, cy - t / 2.0, t, t, Paint::TextOnly).show(node, true),
        );
        if n > 1 {
            let step = 360.0 / n as f64;
            let start = (a.to_degrees() + step * 0.31).rem_euclid(360.0);
            let end = (a.to_degrees() + step * 0.69).rem_euclid(360.0);
            let mut arrow = Drawn::new(
                "circularArrow",
                -r,
                -r,
                d_ring,
                d_ring,
                label("node1", i, n),
            )
            .adj("adj1", 5_202)
            .adj("adj2", 336_015)
            .adj("adj3", (end * 60_000.0).round() as i64)
            .adj("adj4", (start * 60_000.0).round() as i64)
            .adj("adj5", 6_068)
            .group(9);
            arrow.node = Some(node.id.clone());
            out.push(arrow);
        }
    }
    fit_into(out, w, h)
}

fn block_cycle(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let bw = match n {
        1 | 2 => 0.6,
        3 => 0.6,
        4 => 0.5,
        5 => 0.47,
        _ => 2.4 / n as f64,
    };
    let bh = bw / 2.0;
    let r = 0.5;
    let mut out = Vec::new();
    if n > 1 {
        // One arrow round the ring, open where the first box sits.
        let gap = (bw / 2.0 / r).atan().to_degrees();
        let start = (270.0 + gap).rem_euclid(360.0);
        let end = (270.0 - gap).rem_euclid(360.0);
        out.push(
            Drawn::new(
                "circularArrow",
                -r * 1.07,
                -r * 1.07,
                r * 2.14,
                r * 2.14,
                label("bgShp", 0, 1),
            )
            .adj("adj1", 5_544)
            .adj("adj2", 330_680)
            .adj("adj3", (end * 60_000.0).round() as i64)
            .adj("adj4", (start * 60_000.0).round() as i64)
            .adj("adj5", 5_757)
            .group(9),
        );
    }
    for (i, node) in nodes.iter().enumerate() {
        let a = ring(n, i);
        let (cx, cy) = (r * a.cos(), r * a.sin());
        out.push(
            Drawn::new(
                "roundRect",
                cx - bw / 2.0,
                cy - bh / 2.0,
                bw,
                bh,
                label("node1", i, n),
            )
            .show(node, true),
        );
    }
    fit_into(out, w, h)
}

fn radial(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let Some(root) = nodes.first() else {
        return Vec::new();
    };
    let k = root.children.len();
    let d = if k == 0 { 1.0 } else { 0.3384 };
    let r = 0.5 - d / 2.0;
    let mut out = vec![
        Drawn::new("ellipse", -d / 2.0, -d / 2.0, d, d, label("node0", 0, 1)).show(root, false),
    ];
    for (i, child) in root.children.iter().enumerate() {
        let a = ring(k, i);
        let (cx, cy) = (r * a.cos(), r * a.sin());
        let (ix, iy) = (d / 2.0 * a.cos(), d / 2.0 * a.sin());
        let (ox, oy) = ((r - d / 2.0) * a.cos(), (r - d / 2.0) * a.sin());
        let mut line = Drawn::new(
            "line",
            ix.min(ox),
            iy.min(oy),
            (ox - ix).abs(),
            (oy - iy).abs(),
            label("parChTrans1D1", i, k),
        )
        .group(9);
        line.path = Some(vec![(ix, iy), (ox, oy)]);
        line.node = Some(child.id.clone());
        out.push(line);
        out.push(
            Drawn::new(
                "ellipse",
                cx - d / 2.0,
                cy - d / 2.0,
                d,
                d,
                label("node1", i, k),
            )
            .show(child, true),
        );
    }
    fit_into(out, w, h)
}

fn pie(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let d = 1.0;
    let step = 360.0 / n as f64;
    let explode = 0.06 * d;
    let mut out = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let start = (270.0 + i as f64 * step).rem_euclid(360.0);
        let end = (270.0 + (i + 1) as f64 * step).rem_euclid(360.0);
        let mid = (270.0 + (i as f64 + 0.5) * step).to_radians();
        let (dx, dy) = if i == 0 && n > 1 {
            (explode * mid.cos(), explode * mid.sin())
        } else {
            (0.0, 0.0)
        };
        let (tx, ty) = (0.3 * mid.cos() + dx, 0.3 * mid.sin() + dy);
        let tb = 0.3;
        out.push(
            Drawn::new("pie", -0.5 + dx, -0.5 + dy, d, d, label("node1", i, n))
                .adj("adj1", (start * 60_000.0).round() as i64)
                .adj("adj2", (end * 60_000.0).round() as i64)
                .show(node, true)
                .text_box([tx - tb / 2.0, ty - tb / 2.0, tb, tb]),
        );
    }
    fit_into(out, w, h)
}

/// Leaves under `node`, for spreading a tree.
fn leaves(node: &Node) -> usize {
    node.children.iter().map(leaves).sum::<usize>().max(1)
}

/// An elbow connector from a parent's bottom to a child's top, the bend
/// halfway.
fn elbow(from: (f64, f64), to: (f64, f64), paint: Paint, node: &Node) -> Drawn {
    let mid = (from.1 + to.1) / 2.0;
    let path = vec![from, (from.0, mid), (to.0, mid), to];
    let mut line = Drawn::new(
        "line",
        from.0.min(to.0),
        from.1.min(to.1),
        (to.0 - from.0).abs(),
        (to.1 - from.1).abs(),
        paint,
    )
    .group(9);
    line.path = Some(path);
    line.node = Some(node.id.clone());
    line
}

fn org_chart(nodes: &[Node], w: f64, h: f64, titled: bool) -> Vec<Drawn> {
    // Units: a box is 1 wide, 0.5 high; siblings 0.21 apart, levels 0.42
    // apart. Below the second level, bullets hang under their manager.
    let gx = 0.21;
    let mut out = Vec::new();
    fn width(node: &Node, level: usize) -> f64 {
        let staff: Vec<&Node> = node
            .children
            .iter()
            .filter(|c| c.kind == NodeKind::Node)
            .collect();
        if staff.is_empty() {
            return 1.0;
        }
        if level >= 1 {
            // Hanging: one column, a quarter in.
            let deepest = staff
                .iter()
                .map(|c| width(c, level + 1))
                .fold(0.0, f64::max);
            return (0.25 + deepest).max(1.0);
        }
        staff.iter().map(|c| width(c, level + 1)).sum::<f64>() + 0.21 * (staff.len() - 1) as f64
    }
    fn rows(node: &Node, level: usize) -> f64 {
        let staff: Vec<&Node> = node
            .children
            .iter()
            .filter(|c| c.kind == NodeKind::Node)
            .collect();
        let assistants = node
            .children
            .iter()
            .filter(|c| c.kind == NodeKind::Assistant)
            .count();
        let below = if staff.is_empty() {
            0.0
        } else if level >= 1 {
            staff.iter().map(|c| rows(c, level + 1)).sum()
        } else {
            staff.iter().map(|c| rows(c, level + 1)).fold(0.0, f64::max)
        };
        1.0 + if assistants > 0 { 1.0 } else { 0.0 } + below
    }
    #[allow(clippy::too_many_arguments)]
    fn place(
        node: &Node,
        level: usize,
        left: f64,
        top: f64,
        out: &mut Vec<Drawn>,
        titled: bool,
        index: &mut usize,
        count: usize,
    ) {
        let (bw, bh, gx, gy) = (1.0, 0.5, 0.21, 0.21);
        let span = width(node, level);
        let x = if level >= 1 && !node.children.is_empty() {
            left
        } else {
            left + (span - bw) / 2.0
        };
        let label_name = if level == 0 { "node0" } else { "node1" };
        let mut shape =
            Drawn::new("rect", x, top, bw, bh, label(label_name, *index, count)).show(node, false);
        if titled {
            shape.preset = "rect".into();
        }
        *index += 1;
        out.push(shape);
        let bottom = (x + bw / 2.0, top + bh);
        let mut next_top = top + bh + gy;
        let assistants: Vec<&Node> = node
            .children
            .iter()
            .filter(|c| c.kind == NodeKind::Assistant)
            .collect();
        if !assistants.is_empty() {
            for (j, assistant) in assistants.iter().enumerate() {
                let ax = if j % 2 == 0 {
                    x + bw * 0.75 + gx
                } else {
                    x - bw * 0.75 - gx
                };
                out.push(
                    Drawn::new(
                        "rect",
                        ax,
                        next_top,
                        bw,
                        bh,
                        label("asst1", j, assistants.len()),
                    )
                    .show(assistant, false),
                );
                let mut line = Drawn::new(
                    "line",
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    label("parChTrans1D2", j, assistants.len()),
                )
                .group(9);
                let y = next_top + bh / 2.0;
                let edge = if j % 2 == 0 { ax } else { ax + bw };
                line.path = Some(vec![bottom, (bottom.0, y), (edge, y)]);
                line.node = Some(assistant.id.clone());
                out.push(line);
            }
            next_top += bh + gy;
        }
        let staff: Vec<&Node> = node
            .children
            .iter()
            .filter(|c| c.kind == NodeKind::Node)
            .collect();
        if staff.is_empty() {
            return;
        }
        if level >= 1 {
            // Hanging under the manager, a quarter in, joined from the left.
            let stem = x + bw * 0.125;
            for child in staff {
                let child_left = x + 0.25;
                let mut line =
                    Drawn::new("line", 0.0, 0.0, 0.0, 0.0, label("parChTrans1D2", 0, 1)).group(9);
                line.path = Some(vec![
                    (stem, top + bh),
                    (stem, next_top + bh / 2.0),
                    (child_left, next_top + bh / 2.0),
                ]);
                line.node = Some(child.id.clone());
                out.push(line);
                place(
                    child,
                    level + 1,
                    child_left,
                    next_top,
                    out,
                    titled,
                    index,
                    count,
                );
                next_top += rows(child, level + 1) * (bh + gy);
            }
            return;
        }
        let mut child_left = left;
        for child in staff {
            let child_span = width(child, level + 1);
            let child_x = if level + 1 >= 1 && !child.children.is_empty() {
                child_left
            } else {
                child_left + (child_span - bw) / 2.0
            };
            out.push(elbow(
                bottom,
                (child_x + bw / 2.0, next_top),
                label("parChTrans1D2", 0, 1),
                child,
            ));
            place(
                child,
                level + 1,
                child_left,
                next_top,
                out,
                titled,
                index,
                count,
            );
            child_left += child_span + gx;
        }
    }
    let count: usize = nodes.iter().map(Node::count).sum();
    let mut index = 0;
    let mut left = 0.0;
    for node in nodes {
        place(node, 0, left, 0.0, &mut out, titled, &mut index, count);
        left += width(node, 0) + gx;
    }
    // Connectors behind the boxes.
    out.sort_by_key(|shape| if shape.path.is_some() { 0 } else { 1 });
    fit_into(out, w, h)
}

fn hierarchy(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    // Leaves get equal columns; a parent sits over its leaves. Each bullet
    // is a filled box with a lighter one over it, a little down and right.
    let (bw, bh) = (1.0, 0.635);
    let (gx, gy) = (0.222, 0.458 * bh);
    let total_leaves: usize = nodes.iter().map(leaves).sum::<usize>().max(1);
    let count: usize = nodes.iter().map(Node::count).sum();
    let mut out = Vec::new();
    let mut boxes = Vec::new();
    #[allow(clippy::too_many_arguments)]
    fn place(
        node: &Node,
        first: usize,
        level: usize,
        (bw, bh, gx, gy): (f64, f64, f64, f64),
        out: &mut Vec<Drawn>,
        boxes: &mut Vec<Drawn>,
        index: &mut usize,
        count: usize,
    ) -> (f64, f64) {
        let span = leaves(node) as f64;
        let column = bw + gx;
        let cx = column * (first as f64 + span / 2.0);
        let top = level as f64 * (bh + gy + bh * 0.166);
        let x = cx - bw / 2.0;
        let mut back = Drawn::new("roundRect", x, top, bw, bh, label("node0", *index, count))
            .adj("adj", 10_000);
        back.node = Some(node.id.clone());
        boxes.push(back);
        boxes.push(
            Drawn::new(
                "roundRect",
                x + 0.111 * bw,
                top + 0.166 * bh,
                bw,
                bh,
                label("fgAcc1", *index, count),
            )
            .adj("adj", 10_000)
            .show(node, false),
        );
        *index += 1;
        let mut next = first;
        for child in &node.children {
            let (child_x, child_top) = place(
                child,
                next,
                level + 1,
                (bw, bh, gx, gy),
                out,
                boxes,
                index,
                count,
            );
            out.push(elbow(
                (cx, top + bh),
                (child_x, child_top),
                label("parChTrans1D2", 0, 1),
                child,
            ));
            next += leaves(child);
        }
        (cx, top)
    }
    let mut first = 0;
    let mut index = 0;
    for node in nodes {
        place(
            node,
            first,
            0,
            (bw, bh, gx, gy),
            &mut out,
            &mut boxes,
            &mut index,
            count,
        );
        first += leaves(node);
    }
    let _ = total_leaves;
    out.extend(boxes);
    fit_into(out, w, h)
}

fn horizontal_hierarchy(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let (bw, bh) = (1.0, 0.5);
    let (gx, gy) = (0.4, 0.075);
    let count: usize = nodes.iter().map(Node::count).sum();
    let mut out = Vec::new();
    let mut boxes = Vec::new();
    #[allow(clippy::too_many_arguments)]
    fn place(
        node: &Node,
        first: usize,
        level: usize,
        (bw, bh, gx, gy): (f64, f64, f64, f64),
        out: &mut Vec<Drawn>,
        boxes: &mut Vec<Drawn>,
        index: &mut usize,
        count: usize,
    ) -> (f64, f64) {
        let span = leaves(node) as f64;
        let row = bh + gy;
        let cy = row * (first as f64 + span / 2.0);
        let x = level as f64 * (bw + gx);
        let label_name = if level == 0 { "node0" } else { "node1" };
        boxes.push(
            Drawn::new(
                "roundRect",
                x,
                cy - bh / 2.0,
                bw,
                bh,
                label(label_name, *index, count),
            )
            .adj("adj", 10_000)
            .show(node, false),
        );
        *index += 1;
        let mut next = first;
        for child in &node.children {
            let (child_x, child_y) = place(
                child,
                next,
                level + 1,
                (bw, bh, gx, gy),
                out,
                boxes,
                index,
                count,
            );
            let mut line =
                Drawn::new("line", 0.0, 0.0, 0.0, 0.0, label("parChTrans1D2", 0, 1)).group(9);
            line.path = Some(vec![(x + bw, cy), (child_x, child_y)]);
            line.node = Some(child.id.clone());
            out.push(line);
            next += leaves(child);
        }
        (x, cy)
    }
    let mut first = 0;
    let mut index = 0;
    for node in nodes {
        place(
            node,
            first,
            0,
            (bw, bh, gx, gy),
            &mut out,
            &mut boxes,
            &mut index,
            count,
        );
        first += leaves(node);
    }
    out.extend(boxes);
    fit_into(out, w, h)
}

fn hierarchy_list(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let (bw, bh) = (1.0, 0.5);
    let gap = 0.15;
    let mut out = Vec::new();
    let all: usize = nodes.iter().map(|node| node.children.len()).sum();
    let mut child_index = 0;
    for (i, node) in nodes.iter().enumerate() {
        let x = i as f64 * (bw + gap);
        out.push(
            Drawn::new("roundRect", x, 0.0, bw, bh, label("node1", i, n))
                .adj("adj", 10_000)
                .show(node, false),
        );
        let mut y = bh * 1.25;
        for child in &node.children {
            let mut line =
                Drawn::new("line", 0.0, 0.0, 0.0, 0.0, label("parChTrans1D2", 0, 1)).group(9);
            line.path = Some(vec![
                (x + 0.1 * bw, bh),
                (x + 0.1 * bw, y + bh / 2.0),
                (x + 0.2 * bw, y + bh / 2.0),
            ]);
            line.node = Some(child.id.clone());
            out.push(line);
            out.push(
                Drawn::new(
                    "roundRect",
                    x + 0.2 * bw,
                    y,
                    0.8 * bw,
                    bh,
                    label("fgAcc1", child_index, all),
                )
                .adj("adj", 10_000)
                .show(child, true)
                .group(1),
            );
            child_index += 1;
            y += bh * 1.25;
        }
    }
    fit_into(out, w, h)
}

fn venn(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let mut out = Vec::new();
    let d = if n <= 1 {
        1.0
    } else if n <= 3 {
        0.6
    } else {
        0.35
    };
    let r = if n <= 1 {
        0.0
    } else if n <= 3 {
        0.24
    } else {
        0.18
    };
    for (i, node) in nodes.iter().enumerate() {
        let a = ring(n, i);
        let (cx, cy) = (r * a.cos(), r * a.sin());
        let circle = Drawn::new(
            "ellipse",
            cx - d / 2.0,
            cy - d / 2.0,
            d,
            d,
            label("vennNode1", i, n),
        );
        if n <= 3 {
            // The text inside, towards the outside of the overlap.
            let (tx, ty) = (cx + d * 0.12 * a.cos(), cy + d * 0.12 * a.sin());
            out.push(circle.show(node, true).text_box([
                tx - d * 0.3,
                ty - d * 0.25,
                d * 0.6,
                d * 0.5,
            ]));
        } else {
            let mut circle = circle;
            circle.node = Some(node.id.clone());
            out.push(circle);
            let (tx, ty) = ((r + d * 0.85) * a.cos(), (r + d * 0.85) * a.sin());
            out.push(
                Drawn::new(
                    "rect",
                    tx - d * 0.55,
                    ty - d * 0.38,
                    d * 1.1,
                    d * 0.76,
                    Paint::TextOnly,
                )
                .show(node, true),
            );
        }
    }
    fit_into(out, w, h)
}

fn linear_venn(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let d = (w / (1.0 + 0.8 * (n - 1) as f64)).min(h);
    let total = d * (1.0 + 0.8 * (n - 1) as f64);
    let (left, top) = ((w - total) / 2.0, (h - d) / 2.0);
    nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let x = left + i as f64 * d * 0.8;
            Drawn::new("ellipse", x, top, d, d, label("vennNode1", i, n))
                .show(node, true)
                .text_box([x + d * 0.15, top + d * 0.2, d * 0.7, d * 0.6])
        })
        .collect()
}

fn stacked_venn(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let outer = h.min(w);
    let step = if n > 1 {
        outer * 0.6 / (n - 1) as f64
    } else {
        0.0
    };
    nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let d = outer - i as f64 * step;
            let x = (w - d) / 2.0;
            let y = h - d;
            let band = if i + 1 < n {
                step.max(d * 0.12)
            } else {
                d * 0.5
            };
            Drawn::new("ellipse", x, y, d, d, label("node1", i, n))
                .show(node, true)
                .text_box([x + d * 0.25, y + d * 0.02, d * 0.5, band * 0.9])
                .group(0)
        })
        .collect()
}

fn nested_target(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let d0 = h * 0.75;
    let (cx, cy) = (w / 3.0, h - d0 / 2.0);
    let mut circles = Vec::new();
    let mut out = Vec::new();
    let step = d0 / (n as f64 + 0.5);
    let row = h / n.max(1) as f64;
    for (i, node) in nodes.iter().enumerate() {
        let d = d0 - i as f64 * step;
        let mut circle = Drawn::new(
            "ellipse",
            cx - d / 2.0,
            cy - d / 2.0,
            d,
            d,
            label("node1", i, n),
        );
        circle.node = Some(node.id.clone());
        circles.push(circle);
        let ty = i as f64 * row;
        let text_h = row * 0.94;
        out.push(
            Drawn::new("rect", w * 2.0 / 3.0, ty, w / 4.0, text_h, Paint::TextOnly)
                .show(node, true),
        );
        let mut line =
            Drawn::new("line", 0.0, 0.0, 0.0, 0.0, label("parChTrans1D1", i, n)).group(9);
        let inner = d / 2.0 - step / 4.0;
        line.path = Some(vec![
            (w * 2.0 / 3.0, ty + text_h / 2.0),
            (w * 2.0 / 3.0 - w / 16.0, ty + text_h / 2.0),
            (cx + inner * 0.7, cy - inner * 0.7),
        ]);
        line.node = Some(node.id.clone());
        out.push(line);
    }
    circles.extend(out);
    circles
}

fn equation(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let d = (w / (n as f64 + 0.7424 * (n - 1) as f64)).min(h);
    let total = d * (n as f64 + 0.7424 * (n - 1) as f64);
    let (left, top) = ((w - total) / 2.0, (h - d) / 2.0);
    let mut out = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let x = left + i as f64 * d * 1.7424;
        out.push(Drawn::new("ellipse", x, top, d, d, label("node1", i, n)).show(node, true));
        if i + 1 < n {
            let s = d * 0.58;
            let preset = if i + 2 == n { "mathEqual" } else { "mathPlus" };
            let mut op = Drawn::new(
                preset,
                x + d * 1.0807,
                top + (d - s) / 2.0,
                s,
                s,
                label("sibTrans2D1", i, n - 1),
            )
            .group(9);
            op.node = Some(node.id.clone());
            out.push(op);
        }
    }
    out
}

fn matrix(nodes: &[Node], w: f64, h: f64, grid_arrows: bool) -> Vec<Drawn> {
    let side = h.min(w);
    let (left, top) = ((w - side) / 2.0, (h - side) / 2.0);
    let mut out = vec![if grid_arrows {
        Drawn::new("quadArrow", left, top, side, side, label("bgShp", 0, 1))
            .adj("adj1", 2_000)
            .adj("adj2", 4_000)
            .adj("adj3", 5_000)
            .group(9)
    } else {
        Drawn::new("diamond", left, top, side, side, label("bgShp", 0, 1)).group(9)
    }];
    let (s, gap) = if grid_arrows {
        (side * 0.4, side * 0.07)
    } else {
        (side * 0.39, side * 0.03)
    };
    let x0 = w / 2.0 - s - gap / 2.0;
    let y0 = h / 2.0 - s - gap / 2.0;
    for (i, node) in nodes.iter().take(4).enumerate() {
        let (column, row) = (i % 2, i / 2);
        out.push(
            Drawn::new(
                "roundRect",
                x0 + column as f64 * (s + gap),
                y0 + row as f64 * (s + gap),
                s,
                s,
                label("node1", i, 4.min(nodes.len())),
            )
            .show(node, true),
        );
    }
    out
}

fn titled_matrix(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let (qw, qh) = (w / 2.0, h / 2.0);
    let mut out = Vec::new();
    let title = nodes.first();
    let quadrants = title.map(|t| t.children.as_slice()).unwrap_or(&[]);
    // Four quarters, each a rectangle rounded at its outer corner.
    let quarters: [(f64, f64, f64); 4] = [
        (0.0, 0.0, 270.0),
        (qw, 0.0, 0.0),
        (0.0, qh, 180.0),
        (qw, qh, 90.0),
    ];
    for (i, (x, y, rot)) in quarters.iter().enumerate() {
        let mut quarter = if (*rot - 90.0).abs() < 0.1 || (*rot - 270.0).abs() < 0.1 {
            let (cx, cy) = (x + qw / 2.0, y + qh / 2.0);
            Drawn::new(
                "round1Rect",
                cx - qh / 2.0,
                cy - qw / 2.0,
                qh,
                qw,
                label("node1", i, 4),
            )
            .rot(*rot)
        } else {
            Drawn::new("round1Rect", *x, *y, qw, qh, label("node1", i, 4)).rot(*rot)
        };
        let text_box = [x + qw * 0.1, y + qh * 0.1, qw * 0.8, qh * 0.8];
        if let Some(node) = quadrants.get(i) {
            quarter = quarter.show(node, true);
        }
        out.push(quarter.text_box(text_box).group(1));
    }
    if let Some(title) = title {
        out.push(
            Drawn::new(
                "roundRect",
                w * 0.35,
                h * 0.375,
                w * 0.3,
                h * 0.25,
                label("fgShp", 0, 1),
            )
            .show(title, false)
            .group(0),
        );
    }
    out
}

fn pyramid(nodes: &[Node], w: f64, h: f64, inverted: bool) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    // With sub-points the pyramid takes the left 68% and each level's
    // sub-points sit in a band from its slope to the right edge.
    let any_children = nodes.iter().any(|node| !node.children.is_empty());
    let pw = if any_children { w * 0.68 } else { w };
    let lh = h / n as f64;
    let shoulder = pw / (2.0 * n as f64);
    let mut out = Vec::new();
    let mut bands = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let level = if inverted { n - i } else { i + 1 };
        let lw = pw * level as f64 / n as f64;
        let x = (pw - lw) / 2.0;
        let y = i as f64 * lh;
        let adj = (shoulder / lw.min(lh) * 100_000.0).round() as i64;
        if !node.children.is_empty() {
            let bx = x + lw - shoulder;
            let mut band = Drawn::new(
                "nonIsoscelesTrapezoid",
                bx,
                y,
                w - bx,
                lh,
                label("alignAccFollowNode1", i, n),
            )
            .adj("adj1", if inverted { 0 } else { adj })
            .adj("adj2", if inverted { adj } else { 0 })
            .show_children(node)
            .text_box([bx + shoulder, y, w - bx - shoulder, lh])
            .group(1);
            band.node = Some(node.id.clone());
            band.top_left = false;
            bands.push(band);
        }
        let text_box = [x + shoulder * 0.5, y, lw - shoulder, lh];
        if inverted {
            // Turned over, with the text upright in a box of its own.
            let mut level_shape = Drawn::new("trapezoid", x, y, lw, lh, label("node1", i, n))
                .adj("adj", adj)
                .rot(180.0);
            level_shape.node = Some(node.id.clone());
            level_shape.format = node.format.clone();
            out.push(level_shape);
            out.push(
                Drawn::new(
                    "rect",
                    text_box[0],
                    y,
                    text_box[2],
                    lh,
                    Paint::TextOf {
                        label: "node1",
                        index: i,
                        count: n,
                    },
                )
                .show(node, false),
            );
        } else {
            out.push(
                Drawn::new("trapezoid", x, y, lw, lh, label("node1", i, n))
                    .adj("adj", adj)
                    .show(node, false)
                    .text_box(text_box),
            );
        }
    }
    bands.extend(out);
    bands
}

fn pyramid_list(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let side = h.min(w);
    let tri_x = (w - side) / 2.0 - w * 0.05;
    let mut out =
        vec![Drawn::new("triangle", tri_x, 0.0, side, side, label("node1", 0, 1)).group(9)];
    let region = side * 0.782;
    let bh = region / (n as f64 + 0.125 * (n - 1) as f64);
    let bx = tri_x + side / 2.0;
    let bw = (w * 0.4333).min(w - bx);
    for (i, node) in nodes.iter().enumerate() {
        out.push(
            Drawn::new(
                "roundRect",
                bx,
                side * 0.1 + i as f64 * bh * 1.125,
                bw,
                bh,
                label("fgAcc1", i, n),
            )
            .show(node, true),
        );
    }
    out
}

fn segmented_pyramid(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let mut rows = 1;
    while rows * rows < n {
        rows += 1;
    }
    let s = (h / rows as f64).min(w / rows as f64);
    let top = (h - s * rows as f64) / 2.0;
    let cx = w / 2.0;
    let total = rows * rows;
    let mut out = Vec::new();
    let mut index = 0;
    for row in 0..rows {
        let x0 = cx - (row as f64 + 1.0) * s / 2.0;
        for k in 0..(2 * row + 1) {
            let x = x0 + k as f64 * s / 2.0;
            let y = top + row as f64 * s;
            let mut tri = Drawn::new("triangle", x, y, s, s, label("node1", index.min(n - 1), n));
            if k % 2 == 1 {
                tri = tri.rot(180.0);
            }
            if let Some(node) = nodes.get(index) {
                if k % 2 == 1 {
                    tri.node = Some(node.id.clone());
                    tri.format = node.format.clone();
                    out.push(tri);
                    out.push(
                        Drawn::new(
                            "rect",
                            x + s * 0.25,
                            y + s * 0.1,
                            s * 0.5,
                            s * 0.45,
                            Paint::TextOf {
                                label: "node1",
                                index,
                                count: n,
                            },
                        )
                        .show(node, true),
                    );
                } else {
                    out.push(tri.show(node, true).text_box([
                        x + s * 0.25,
                        y + s * 0.45,
                        s * 0.5,
                        s * 0.45,
                    ]));
                }
            } else {
                out.push(tri);
            }
            index += 1;
        }
    }
    let _ = total;
    out
}

fn picture_captions(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len();
    let (columns, cw) = grid(n, w, h, 1.06, 0.1, 0.1 / 1.06 * 1.06);
    let (ih, th) = (cw * 0.689, cw * 0.371);
    let gap = cw * 0.1;
    let mut out = Vec::new();
    for (i, ((x, y), node)) in grid_cells(n, columns, (cw, ih + th), (gap, gap), w, h)
        .into_iter()
        .zip(nodes)
        .enumerate()
    {
        out.push(
            Drawn::new("roundRect", x, y, cw, ih, label("node1", i, n))
                .picture(node)
                .group(9),
        );
        out.push(Drawn::new("rect", x, y + ih, cw, th, Paint::TextOnly).show(node, true));
    }
    out
}

fn titled_pictures(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len();
    let (columns, cw) = grid(n, w, h, 1.0089, 0.4934, 0.134 / 1.0089 * 1.0089);
    let (title, gap, pic) = (cw * 0.1459, cw * 0.0157, cw * 0.8473);
    let mut out = Vec::new();
    for (i, ((x, y), node)) in grid_cells(
        n,
        columns,
        (cw, title + gap + pic),
        (cw * 0.4934, cw * 0.134),
        w,
        h,
    )
    .into_iter()
    .zip(nodes)
    .enumerate()
    {
        out.push(
            Drawn::new(
                "rect",
                x,
                y + title + gap,
                cw,
                pic,
                label("fgImgPlace1", i, n),
            )
            .picture(node)
            .group(9),
        );
        out.push(
            Drawn::new("rect", x, y, cw, title, label("node1", i, n))
                .show(node, true)
                .group(0),
        );
    }
    out
}

fn picture_strips(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len().max(1);
    let columns = if n > 1 { 2 } else { 1 };
    let rows = n.div_ceil(columns);
    let cw = w * 0.464;
    let ch = (cw * 0.3125).min(h / (rows as f64 * 1.26));
    let pitch_y = ch * 1.259;
    let gap_x = w * 0.0516;
    let cells = grid_cells(n, columns, (cw, ch), (gap_x, pitch_y - ch), w, h);
    let mut out = Vec::new();
    for (i, ((x, y), node)) in cells.into_iter().zip(nodes).enumerate() {
        out.push(
            Drawn::new("rect", x, y, cw, ch, label("trAlignAcc1", i, n))
                .show(node, true)
                .text_box([x + cw * 0.22, y, cw * 0.78, ch]),
        );
        out.push(
            Drawn::new(
                "rect",
                x - cw * 0.0417,
                y - ch * 0.1444,
                cw * 0.2188,
                ch * 1.05,
                label("fgImgPlace1", i, n),
            )
            .picture(node)
            .group(9),
        );
    }
    out
}

fn captioned_pictures(nodes: &[Node], w: f64, h: f64) -> Vec<Drawn> {
    let n = nodes.len();
    let (columns, cw) = grid(n, w, h, 1.1765, 0.1765, 0.085);
    let ch = cw * 1.1765;
    let mut out = Vec::new();
    for (i, ((x, y), node)) in grid_cells(n, columns, (cw, ch), (cw * 0.1765, cw * 0.1), w, h)
        .into_iter()
        .zip(nodes)
        .enumerate()
    {
        let mut frame = Drawn::new("rect", x, y, cw, ch, label("trAlignAcc1", i, n)).group(9);
        frame.node = Some(node.id.clone());
        out.push(frame);
        out.push(
            Drawn::new(
                "rect",
                x + cw * 0.05,
                y + cw * 0.047,
                cw * 0.9,
                cw * 0.7647,
                label("fgImgPlace1", i, n),
            )
            .picture(node)
            .group(9),
        );
        out.push(
            Drawn::new(
                "rect",
                x + cw * 0.05,
                y + cw * 0.8117,
                cw * 0.9,
                cw * 0.3176,
                Paint::TextOnly,
            )
            .show(node, true),
        );
    }
    out
}

/// The width of `text` in ems at 1 pt: CJK and full-width forms a whole
/// em, Latin about half.
fn ems(text: &str) -> f64 {
    text.chars()
        .map(|ch| {
            let c = ch as u32;
            if (0x3000..=0x9FFF).contains(&c)
                || (0xAC00..=0xD7AF).contains(&c)
                || (0xFF00..=0xFFEF).contains(&c)
            {
                1.0
            } else if ch.is_ascii_uppercase() || ch == 'm' || ch == 'w' {
                0.62
            } else if ch == ' ' || ch == 'i' || ch == 'l' || ch == '.' {
                0.27
            } else {
                0.5
            }
        })
        .sum()
}

/// Lines `text` wraps to in `width` pt at `size` pt.
fn wrapped_lines(text: &str, size: f64, width: f64) -> usize {
    if width <= 0.0 {
        return usize::MAX / 4;
    }
    let mut lines = 1;
    let mut line = 0.0;
    let mut word = 0.0;
    let flush = |line: &mut f64, word: &mut f64, lines: &mut usize| {
        if *line > 0.0 && *line + *word > width {
            *lines += 1;
            *line = 0.0;
        }
        // A word longer than the line breaks within itself.
        while *word > width {
            *word -= width;
            *lines += 1;
        }
        *line += *word;
        *word = 0.0;
    };
    for ch in text.chars() {
        let advance = ems(ch.encode_utf8(&mut [0; 4])) * size;
        if ch == ' ' {
            flush(&mut line, &mut word, &mut lines);
            line += advance;
        } else if advance >= size * 0.99 {
            // CJK breaks anywhere.
            flush(&mut line, &mut word, &mut lines);
            if line > 0.0 && line + advance > width {
                lines += 1;
                line = 0.0;
            }
            line += advance;
        } else {
            word += advance;
        }
    }
    flush(&mut line, &mut word, &mut lines);
    lines
}

/// Whether `shape`'s text fits at `size` pt.
fn fits(shape: &Drawn, size: f64) -> bool {
    const EMU_PER_PT: f64 = 12_700.0;
    let [_, _, bw, bh] = shape
        .text_box
        .unwrap_or([shape.x, shape.y, shape.w, shape.h]);
    let (bw, bh) = if (shape.rot - 90.0).abs() < 0.1 && shape.text_box.is_none() {
        (bh, bw)
    } else {
        (bw, bh)
    };
    let inset = shape.inset * size;
    let width = bw / EMU_PER_PT - 2.0 * inset;
    let height = bh / EMU_PER_PT - 2.0 * inset;
    let mut used = 0.0;
    for para in &shape.paras {
        let s = if para.level > 0 || shape.bullets {
            size * 0.78
        } else {
            size
        };
        let indent = if para.level > 0 || shape.bullets {
            s * 0.6
        } else {
            0.0
        };
        let lines = wrapped_lines(&para.text, s, width - indent) as f64;
        used += lines * s * 1.2 * 0.9 + s * if para.level > 0 { 0.15 } else { 0.35 } * 1.2;
    }
    used <= height + size * 0.35
}

/// Sizes text: per group, the largest whole point size up to the group's
/// limit at which every shape of it holds its text.
fn fit_text(shapes: &mut [Drawn]) {
    let mut groups: Vec<usize> = shapes.iter().map(|s| s.group).collect();
    groups.sort_unstable();
    groups.dedup();
    for group in groups {
        let members: Vec<usize> = (0..shapes.len())
            .filter(|&i| {
                shapes[i].group == group && shapes[i].paras.iter().any(|p| !p.text.is_empty())
            })
            .collect();
        let max = shapes
            .iter()
            .filter(|s| s.group == group)
            .map(|s| s.max_pt)
            .fold(65.0, f64::min);
        let mut size = max;
        while size > 5.0 && !members.iter().all(|&i| fits(&shapes[i], size)) {
            size -= 1.0;
        }
        for shape in shapes.iter_mut().filter(|s| s.group == group) {
            shape.size_pt = size;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Node> {
        let mut nodes = vec![
            Node::with("A", vec![Node::new("A1"), Node::new("A2")]),
            Node::with("B", vec![Node::new("B1")]),
            Node::new("C"),
        ];
        let mut id = 0;
        fn number(nodes: &mut [Node], id: &mut usize) {
            for node in nodes {
                *id += 1;
                node.id = format!("{{{id}}}");
                number(&mut node.children, id);
            }
        }
        number(&mut nodes, &mut id);
        nodes
    }

    #[test]
    fn every_layout_keeps_its_shapes_in_the_frame_and_shows_every_bullet() {
        let (w, h) = (8_128_000.0, 5_418_667.0);
        for info in catalog::LAYOUTS.iter() {
            let shapes = lay_out(&info.id(), None, &sample(), w, h);
            assert!(!shapes.is_empty(), "{}", info.short);
            for shape in &shapes {
                let (x0, y0, x1, y1) = bounds(std::slice::from_ref(shape)).unwrap();
                assert!(
                    x0 > -w * 0.02 && y0 > -h * 0.02 && x1 < w * 1.02 && y1 < h * 1.02,
                    "{} spills: {shape:?}",
                    info.short
                );
            }
            let text: Vec<&str> = shapes
                .iter()
                .flat_map(|s| s.paras.iter().map(|p| p.text.as_str()))
                .collect();
            // Radial and titled matrix layouts show the first bullet's tree.
            let words: &[&str] = match info.short {
                "radial1" | "matrix1" => &["A", "A1", "A2"],
                "matrix3" | "matrix2" => &["A", "B", "C"],
                _ => &["A", "A1", "A2", "B", "B1", "C"],
            };
            for word in words {
                assert!(text.contains(word), "{} lost {word}: {text:?}", info.short);
            }
        }
    }

    #[test]
    fn text_shrinks_to_fit_and_stays_one_size_across_shapes() {
        let shapes = lay_out("process1", None, &sample(), 8_128_000.0, 5_418_667.0);
        let sizes: Vec<f64> = shapes
            .iter()
            .filter(|s| s.group == 0)
            .map(|s| s.size_pt)
            .collect();
        assert!(sizes.windows(2).all(|pair| pair[0] == pair[1]));
        // PowerPoint draws this at 23 pt.
        assert!((18.0..=28.0).contains(&sizes[0]), "{sizes:?}");
    }
}
