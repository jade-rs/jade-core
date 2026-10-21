//! A native document is laid out with its styles, as the browser lays it out.
//!
//! The renderer takes the style cascade from the document it is synced with,
//! not from the body it renders. A host that only installed the header and
//! footer stories laid every run out with its own and its paragraph's direct
//! formatting: `numbering-restart.docx`'s Normal style tightens its text by
//! 1pt (`w:spacing w:val="-20"`), and natively the glyphs stood 1pt apart.

mod common;
use common::{open_bytes, repo_path};
use std::io::{Cursor, Read, Write};

/// The fixture, its styles part rewritten by `styles`.
fn with_styles(styles: impl FnOnce(String) -> String) -> Vec<u8> {
    let bytes = std::fs::read(repo_path("public/fixtures/numbering-restart.docx")).unwrap();
    let mut input = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut transform = Some(styles);
    for index in 0..input.len() {
        let mut part = input.by_index(index).unwrap();
        let name = part.name().to_string();
        let mut data = Vec::new();
        part.read_to_end(&mut data).unwrap();
        if name == "word/styles.xml" {
            data = transform.take().unwrap()(String::from_utf8(data).unwrap()).into_bytes();
        }
        output
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        output.write_all(&data).unwrap();
    }
    assert!(transform.is_none(), "the fixture has a styles part");
    output.finish().unwrap().into_inner()
}

/// Where each glyph of the first line reading `Article` starts, in px.
fn article_glyph_starts(bytes: &[u8]) -> Vec<f64> {
    let document = open_bytes(bytes);
    let line = document
        .renderer()
        .layout_debug_lines()
        .into_iter()
        .find(|line| line.text.contains("Article"))
        .expect("a line reading Article");
    let fragment = line
        .fragments
        .iter()
        .find(|fragment| fragment.glyphs.iter().any(|glyph| glyph.text == "A"))
        .expect("the fragment holding Article");
    fragment
        .glyphs
        .iter()
        .skip_while(|glyph| glyph.text != "A")
        .take(7)
        .map(|glyph| fragment.x + glyph.x)
        .collect()
}

#[test]
fn a_style_character_spacing_sets_the_glyphs_closer() {
    const SPACING: &str = r#"<w:spacing w:val="-20"/>"#;
    let styled = std::fs::read(repo_path("public/fixtures/numbering-restart.docx")).unwrap();
    // Normal's spacing alone: the run itself names no spacing.
    let unstyled = with_styles(|xml| {
        let normal = xml.find(r#"w:styleId="a">"#).expect("the Normal style");
        let end = normal + xml[normal..].find("</w:style>").unwrap();
        let spacing = normal + xml[normal..end].find(SPACING).expect("Normal's spacing");
        format!("{}{}", &xml[..spacing], &xml[spacing + SPACING.len()..])
    });
    let tight = article_glyph_starts(&styled);
    let natural = article_glyph_starts(&unstyled);
    assert_eq!(tight.len(), 7);
    assert_eq!(natural.len(), 7);
    // Every advance inside `Article` loses the style's 1pt (4/3 px).
    for index in 1..tight.len() {
        let tightened = (natural[index] - natural[index - 1]) - (tight[index] - tight[index - 1]);
        assert!(
            (tightened - 4.0 / 3.0).abs() < 0.01,
            "advance {index} tightened by {tightened}px: {tight:?} against {natural:?}"
        );
    }
}
