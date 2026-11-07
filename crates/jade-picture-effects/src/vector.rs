//! An SVG's drawing as filled and stroked paths, for 図形に変換: each path
//! in the SVG's own units with its transforms applied.

/// One step of a path.
#[derive(Clone, Debug, PartialEq)]
pub enum PathCommand {
    Move(f32, f32),
    Line(f32, f32),
    Quad(f32, f32, f32, f32),
    Cubic(f32, f32, f32, f32, f32, f32),
    Close,
}

/// A path of an SVG with its paint.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorPath {
    pub commands: Vec<PathCommand>,
    /// Fill colour and opacity.
    pub fill: Option<([u8; 3], f32)>,
    /// Stroke colour, opacity and width.
    pub stroke: Option<([u8; 3], f32, f32)>,
}

fn color(paint: &resvg::usvg::Paint) -> [u8; 3] {
    use resvg::usvg::Paint;
    let picked = match paint {
        Paint::Color(color) => Some(*color),
        Paint::LinearGradient(gradient) => gradient.stops().first().map(|stop| stop.color()),
        Paint::RadialGradient(gradient) => gradient.stops().first().map(|stop| stop.color()),
        Paint::Pattern(_) => None,
    };
    picked.map_or([0, 0, 0], |c| [c.red, c.green, c.blue])
}

fn walk(group: &resvg::usvg::Group, out: &mut Vec<VectorPath>) {
    use resvg::usvg::Node;
    for node in group.children() {
        match node {
            Node::Group(group) => walk(group, out),
            Node::Text(text) => walk(text.flattened(), out),
            Node::Path(path) => {
                if !path.is_visible() {
                    continue;
                }
                let Some(data) = path.data().clone().transform(path.abs_transform()) else {
                    continue;
                };
                let commands = data
                    .segments()
                    .map(|segment| {
                        use resvg::tiny_skia::PathSegment;
                        match segment {
                            PathSegment::MoveTo(p) => PathCommand::Move(p.x, p.y),
                            PathSegment::LineTo(p) => PathCommand::Line(p.x, p.y),
                            PathSegment::QuadTo(a, p) => PathCommand::Quad(a.x, a.y, p.x, p.y),
                            PathSegment::CubicTo(a, b, p) => {
                                PathCommand::Cubic(a.x, a.y, b.x, b.y, p.x, p.y)
                            }
                            PathSegment::Close => PathCommand::Close,
                        }
                    })
                    .collect();
                out.push(VectorPath {
                    commands,
                    fill: path
                        .fill()
                        .map(|fill| (color(fill.paint()), fill.opacity().get())),
                    stroke: path.stroke().map(|stroke| {
                        (
                            color(stroke.paint()),
                            stroke.opacity().get(),
                            stroke.width().get(),
                        )
                    }),
                });
            }
            Node::Image(_) => {}
        }
    }
}

/// An SVG's size and its paths, back to front.
pub fn svg_paths(bytes: &[u8]) -> Option<((f32, f32), Vec<VectorPath>)> {
    let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
    let mut paths = Vec::new();
    walk(tree.root(), &mut paths);
    Some(((tree.size().width(), tree.size().height()), paths))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_svgs_paths_come_out_transformed_with_their_paint() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><g transform="translate(10 0)"><rect width="5" height="5" fill="#ff0000" stroke="#0000ff" stroke-width="2"/></g><circle cx="3" cy="3" r="2" fill="none" stroke="#000"/></svg>"##;
        let ((w, h), paths) = svg_paths(svg).unwrap();
        assert_eq!((w, h), (20.0, 10.0));
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].commands[0], PathCommand::Move(10.0, 0.0));
        assert_eq!(paths[0].fill, Some(([255, 0, 0], 1.0)));
        assert_eq!(paths[0].stroke, Some(([0, 0, 255], 1.0, 2.0)));
        assert!(paths[1].fill.is_none());
    }
}
