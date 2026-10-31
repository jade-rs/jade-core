//! Gradient fills as Excel writes them (Format Cells ▸ Fill ▸ Fill
//! Effects): a linear one at an angle and a path one from the centre.

use jade_xlsx_reader::styles::parse_styles;
use jade_xlsx_reader::Color;

const STYLES: &str = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><sz val="11"/></font></fonts><fills count="4"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><gradientFill degree="90"><stop position="0"><color rgb="FFFFFFFF"/></stop><stop position="1"><color rgb="FF4472C4"/></stop></gradientFill></fill><fill><gradientFill type="path" left="0.5" right="0.5" top="0.5" bottom="0.5"><stop position="0"><color theme="0"/></stop><stop position="1"><color theme="4" tint="-0.25"/></stop></gradientFill></fill></fills><borders count="1"><border/></borders><cellXfs count="1"><xf fillId="0"/></cellXfs></styleSheet>"#;

#[test]
fn linear_and_path_gradients_are_read_with_their_stops() {
    let table = parse_styles(STYLES).expect("styles");
    assert_eq!(table.fills.len(), 4);
    assert!(table.fills[1].gradient.is_none());

    let linear = table.fills[2].gradient.as_ref().expect("linear gradient");
    assert_eq!(linear.degree, 90.0);
    assert!(linear.path.is_none());
    assert_eq!(
        linear.stops,
        [
            (0.0, Color::Argb([0xFF, 0xFF, 0xFF, 0xFF])),
            (1.0, Color::Argb([0xFF, 0x44, 0x72, 0xC4])),
        ]
    );

    let path = table.fills[3].gradient.as_ref().expect("path gradient");
    assert_eq!(path.path, Some([0.5, 0.5, 0.5, 0.5]));
    assert_eq!(path.stops.len(), 2);
    assert!(matches!(path.stops[1].1, Color::Theme { index: 4, .. }));
}
