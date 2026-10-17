use jade_skia_painter::SkiaPainter;

#[test]
fn svg_images_use_the_same_source_crop_and_destination_mapping_as_raster_images() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50" viewBox="0 0 100 50"><rect width="50" height="50" fill="#ff0000"/><rect x="50" width="50" height="50" fill="#0000ff"/></svg>"##;
    let painter = SkiaPainter::new(120, 80).unwrap();
    assert!(painter.draw_encoded_image_rect(svg, None, 10.0, 10.0, 100.0, 50.0));
    assert_eq!(painter.pixel(20, 20).unwrap(), (255, 0, 0, 255));
    assert_eq!(painter.pixel(90, 20).unwrap(), (0, 0, 255, 255));
    let painter = SkiaPainter::new(120, 80).unwrap();
    assert!(painter.draw_encoded_image_rect(
        svg,
        Some([0.5, 0.0, 0.5, 1.0]),
        10.0,
        10.0,
        100.0,
        50.0
    ));
    assert_eq!(painter.pixel(20, 20).unwrap(), (0, 0, 255, 255));
    assert_eq!(painter.pixel(5, 20).unwrap(), (255, 255, 255, 255));
    assert!(!painter.draw_encoded_image(b"invalid image", 0.0, 0.0, 10.0, 10.0));
}
