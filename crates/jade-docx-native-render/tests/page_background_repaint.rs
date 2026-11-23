//! A page colour changed in place paints what opening the document with
//! that colour paints, to the last pixel row.
//!
//! An A4 page is 1122.5 CSS px tall, so its last pixel row is half inside
//! the page. Clearing only the page box left half of that row's old pixels
//! under the new colour, and a repaint differed from a fresh paint there.

mod common;
use common::{open_bytes, open_fixture};
use jade_docx_headless_api::HeadlessCommand;

#[test]
fn a_page_colour_set_in_place_paints_as_a_fresh_open() {
    let mut document = open_fixture("public/fixtures/table-complex-pagination.docx");
    for color in [Some("#FFEECC"), Some("#203040"), None] {
        document
            .apply_command(
                HeadlessCommand::SetPageBackgroundColor {
                    color: color.map(str::to_string),
                },
                false,
            )
            .expect("set the page colour");
        let fresh = open_bytes(&document.export_docx().expect("export"));
        assert_eq!(document.page_count(), fresh.page_count());
        for page in 0..document.page_count() {
            let (ours, theirs) = (
                document.page_pixels(page).expect("pixels"),
                fresh.page_pixels(page).expect("pixels"),
            );
            assert_eq!(ours.1, theirs.1);
            let differing: Vec<usize> = ours
                .0
                .chunks(4)
                .zip(theirs.0.chunks(4))
                .enumerate()
                .filter(|(_, (a, b))| a != b)
                .map(|(index, _)| index)
                .collect();
            assert!(
                differing.is_empty(),
                "{color:?}: page {page} differs from a fresh open in {} px, first at row {}",
                differing.len(),
                differing[0] as i32 / ours.1
            );
        }
    }
}
