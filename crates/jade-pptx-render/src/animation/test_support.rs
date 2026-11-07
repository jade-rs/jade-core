//! Small layouts for the animation tests.

use jade_pptx_layout::{PlacedShape, SlideLayout};

pub(crate) fn shape(id: u32, name: &str, x: f64, fill: &str) -> PlacedShape {
    let mut shape: PlacedShape = serde_json::from_str(&format!(
        r#"{{"name":"{name}","rect":{{"x":{x},"y":0.0,"width":100.0,"height":50.0}},"rotation_rad":0.0,"flip_h":false,"flip_v":false,"geometry":"Rect","fill":"{fill}","outline":null,"image_part":null,"lines":[]}}"#
    ))
    .expect("a shape");
    shape.source_id = Some(id);
    shape
}

pub(crate) fn layout(shapes: Vec<PlacedShape>) -> SlideLayout {
    let mut layout: SlideLayout = serde_json::from_str(
        r##"{"width_px":1000.0,"height_px":500.0,"background":"#FFFFFF","shapes":[]}"##,
    )
    .expect("a layout");
    layout.shapes = shapes;
    layout
}
