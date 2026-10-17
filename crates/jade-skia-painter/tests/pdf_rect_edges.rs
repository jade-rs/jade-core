//! Fractional paragraph backgrounds must retain their raster edge coverage.
use jade_painter::Painter;
use jade_skia_painter::SkiaPainter;

#[test]
#[ignore = "requires JADE_PDFTOPPM (Poppler); CI runs this explicitly"]
fn fractional_background_does_not_erase_the_border_beneath_its_edge() {
    let raster = SkiaPainter::new(80, 60).unwrap();
    let recording = SkiaPainter::new_recording(80.0, 60.0).unwrap();
    for painter in [&raster, &recording] {
        painter.set_stroke_style_str("#000000");
        painter.set_line_width(1.0);
        painter.begin_path();
        painter.move_to(8.0, 25.5);
        painter.line_to(72.0, 25.5);
        painter.stroke();
        painter.set_fill_style_str("#ffffff");
        painter.fill_rect(8.4, 25.33334, 63.2, 18.2);
    }
    let directory = std::env::temp_dir().join(format!("jade-pdf-rect-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let pdf_path = directory.join("edge.pdf");
    std::fs::write(&pdf_path, SkiaPainter::encode_pdf(&[recording]).unwrap()).unwrap();
    let status = std::process::Command::new(std::env::var_os("JADE_PDFTOPPM").unwrap())
        .args(["-png", "-singlefile", "-r", "96"])
        .arg(&pdf_path)
        .arg(directory.join("edge"))
        .status()
        .unwrap();
    assert!(status.success());
    let actual = SkiaPainter::new(80, 60).unwrap();
    assert!(actual.draw_encoded_image_rect(
        &std::fs::read(directory.join("edge.png")).unwrap(),
        None,
        0.0,
        0.0,
        80.0,
        60.0
    ));
    let expected = raster.pixel(40, 25).unwrap().0;
    let observed = actual.pixel(40, 25).unwrap().0;
    assert!(expected < 192, "the live painter leaves a visible edge");
    assert!(
        expected.abs_diff(observed) <= 2,
        "PDF erased the border: live={expected}, PDF={observed}; artifacts: {}",
        directory.display()
    );
    std::fs::remove_dir_all(directory).unwrap();
}
