//! 形式を選択して貼り付け → HTML 形式 for a copy from another application:
//! the clipboard's `text/html` read as Word reads a paste of it —
//! paragraphs from the block elements and `br`, the runs' bold, italic,
//! underline, strikethrough, super/subscript, size, face and colour from
//! the tags and the inline styles, entities decoded, white space collapsed.

use jade_docx_editor::word::clipboard::RichClip;

fn texts(clip: &RichClip) -> Vec<String> {
    clip.paragraphs
        .iter()
        .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
        .collect()
}

#[test]
fn blocks_and_line_breaks_are_paragraphs() {
    let clip = RichClip::from_html(
        "<html><head><style>p{color:red}</style><title>x</title></head><body>\
         <p>First   paragraph</p>\n<div>Second<br>third</div><ul><li>one</li><li>two</li></ul>\
         <!-- a comment --></body></html>",
    );
    assert_eq!(
        texts(&clip),
        ["First paragraph", "Second", "third", "one", "two"]
    );
}

#[test]
fn the_tags_and_inline_styles_format_the_runs() {
    let clip = RichClip::from_html(
        "<p style=\"text-align:center\">plain <b>bold</b> <i>italic</i> <u>under</u> \
         <s>gone</s> H<sub>2</sub>O x<sup>2</sup></p>\
         <p><span style=\"font-weight:700; font-size:16px; color:#c00; \
         font-family:'Yu Gothic', sans-serif\">styled</span> \
         <span style=\"color: rgb(0, 112, 192); text-decoration: underline\">blue</span></p>",
    );
    assert_eq!(clip.paragraphs.len(), 2);
    let first = &clip.paragraphs[0];
    assert_eq!(first.alignment.as_deref(), Some("center"));
    let style_of = |paragraph: &jade_docx_editor::word::clipboard::ClipParagraph, text: &str| {
        paragraph
            .runs
            .iter()
            .find(|run| run.text == text)
            .unwrap_or_else(|| panic!("no run {text:?} in {:?}", paragraph.runs))
            .style
            .clone()
    };
    assert_eq!(style_of(first, "bold").bold, Some(true));
    assert_eq!(style_of(first, "italic").italic, Some(true));
    assert_eq!(style_of(first, "under").underline, Some(true));
    assert_eq!(style_of(first, "gone").strike, Some(true));
    assert_eq!(style_of(first, "2").baseline.as_deref(), Some("subscript"));
    assert!(first
        .runs
        .iter()
        .any(|run| run.style.baseline.as_deref() == Some("superscript")));
    let second = &clip.paragraphs[1];
    let styled = style_of(second, "styled");
    assert_eq!(styled.bold, Some(true));
    assert_eq!(styled.font_size, Some(12.0));
    assert_eq!(styled.color.as_deref(), Some("CC0000"));
    assert_eq!(styled.font_family.as_deref(), Some("Yu Gothic"));
    let blue = style_of(second, "blue");
    assert_eq!(blue.color.as_deref(), Some("0070C0"));
    assert_eq!(blue.underline, Some(true));
    assert_eq!(
        clip.plain_text(),
        "plain bold italic under gone H2O x2\nstyled blue"
    );
}

#[test]
fn entities_fragments_and_office_markup() {
    let clip = RichClip::from_html(
        "<html><body>ignored<!--StartFragment--><p class=MsoNormal>A &amp; B &lt;C&gt; \
         &#x3042;&#12356;&nbsp;end<o:p></o:p></p><script>alert(1)</script>\
         <table><tr><td>a</td><td>b</td></tr></table><!--EndFragment-->after</body></html>",
    );
    assert_eq!(texts(&clip), ["A & B <C> あい end", "a\tb"]);
}

#[test]
fn headings_are_bold_and_larger() {
    let clip = RichClip::from_html("<h1>Title</h1><p>body</p>");
    let title = &clip.paragraphs[0].runs[0].style;
    assert_eq!(title.bold, Some(true));
    assert_eq!(title.font_size, Some(24.0));
    assert_eq!(clip.paragraphs[1].runs[0].style.bold, None);
}
