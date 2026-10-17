use jade_painter::{ImageRect, ImageSource, Painter};
use jade_skia_painter::SkiaPainter;
#[test]
fn svg_images_respect_the_saved_global_opacity_and_normalized_source() {
    let painter = SkiaPainter::new(100, 100).unwrap();
    painter.save();
    painter.set_global_alpha(0.5);
    assert!(painter.draw_image(ImageSource{key:"red-svg",bytes:br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#ff0000"/></svg>"##},Some([0.5,0.0,0.5,1.0]),ImageRect{x:10.0,y:10.0,width:80.0,height:80.0}).unwrap());
    let (r, g, b, a) = painter.pixel(50, 50).unwrap();
    assert_eq!((r, a), (255, 255));
    assert!((126..=129).contains(&g) && g == b);
    painter.restore();
    painter.set_fill_style_str("#0000ff");
    painter.fill_rect(0.0, 0.0, 5.0, 5.0);
    assert_eq!(painter.pixel(2, 2), Some((0, 0, 255, 255)));
}
