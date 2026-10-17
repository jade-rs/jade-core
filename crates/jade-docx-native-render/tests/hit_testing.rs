//! Can a click be turned into a text position, natively?
//!
//! Hit testing reads the layout the renderer built, so it should work off the
//! browser — but "should" is what the last three findings in this crate were.
//! This checks it before any UI is built on top of it.

mod common;
use common::open_contract;

#[test]
fn a_point_in_the_text_resolves_to_a_position() {
    let document = open_contract();

    // Well inside the body: past the left margin, below the first line.
    let position = document.position_at(200.0, 200.0);

    assert!(
        position.is_some(),
        "no position under a point in the middle of the text"
    );
}

/// The position has to move with the point, or it is a constant dressed up as
/// a hit test.
#[test]
fn lower_points_resolve_to_later_paragraphs() {
    let document = open_contract();

    let mut seen: Vec<(f64, usize)> = Vec::new();
    for y in [160.0, 260.0, 360.0, 460.0, 560.0] {
        if let Some((paragraph, _)) = document.position_at(200.0, y) {
            seen.push((y, paragraph));
        }
    }

    assert!(
        seen.len() >= 3,
        "only {} of 5 points resolved: {seen:?}",
        seen.len()
    );
    assert!(
        seen.windows(2).any(|w| w[1].1 > w[0].1),
        "paragraph never advances going down the page: {seen:?}"
    );
}

/// Points further right in a line should land at later offsets in it.
#[test]
fn points_further_right_resolve_to_later_offsets() {
    let document = open_contract();

    let left = document.position_at(120.0, 260.0);
    let right = document.position_at(600.0, 260.0);

    match (left, right) {
        (Some(left), Some(right)) if left.0 == right.0 => {
            assert!(
                right.1 > left.1,
                "same paragraph but the offset did not advance: {left:?} -> {right:?}"
            );
        }
        (Some(_), Some(_)) => {
            // Different paragraphs is a legitimate outcome for a short line;
            // the previous test covers vertical movement.
        }
        other => panic!("a point in the body resolved to nothing: {other:?}"),
    }
}
