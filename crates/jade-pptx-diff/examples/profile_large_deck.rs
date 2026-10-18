//! 大きめのデッキで差分計算の時間を測る。
//!
//! ```sh
//! cargo run --release -p structural-pptx-diff --example profile_large_deck -- 400
//! ```
//!
//! 引数はスライド枚数 (既定 400)。IR を直接組み立てるので、parse の時間は含まない。

use std::time::Instant;

use pptx_diff::{
    diff_presentations, DiffOptions, FormatSpanIr, Frame, ParagraphIr, Placeholder, PresentationIr,
    ShapeIr, ShapeKind, SlideIr, TableIr,
};

fn paragraph(text: String) -> ParagraphIr {
    ParagraphIr {
        format_spans: vec![FormatSpanIr {
            characters: text.chars().count(),
            style: String::new(),
        }],
        text,
        level: 0,
        bold: false,
        size: None,
        align: None,
    }
}

fn text_shape(id: usize, placeholder: Option<&str>, paragraphs: Vec<String>) -> ShapeIr {
    ShapeIr {
        id: id.to_string(),
        name: format!("Shape {id}"),
        kind: ShapeKind::Text,
        placeholder: placeholder.map(|kind| Placeholder {
            kind: kind.to_string(),
            index: None,
        }),
        frame: Some(Frame {
            x: 457_200 * id as i64,
            y: 457_200,
            width: 1_828_800,
            height: 914_400,
        }),
        z: id,
        appearance_signature: String::new(),
        paragraphs: paragraphs.into_iter().map(paragraph).collect(),
        table: None,
        image: None,
    }
}

fn table_shape(id: usize, rows: Vec<Vec<String>>) -> ShapeIr {
    let columns = rows.first().map_or(0, Vec::len);
    ShapeIr {
        id: id.to_string(),
        name: format!("Table {id}"),
        kind: ShapeKind::Table,
        placeholder: None,
        frame: None,
        z: id,
        appearance_signature: String::new(),
        paragraphs: Vec::new(),
        table: Some(TableIr {
            column_widths: vec![914_400; columns],
            row_heights: vec![370_840; rows.len()],
            rows,
        }),
        image: None,
    }
}

/// `variant` が違うと本文の一部と表の列が変わる。
fn deck(slides: usize, variant: bool) -> PresentationIr {
    let long = |seed: usize| -> String {
        (0..400)
            .map(|word| format!("第{seed}章の本文その{word}。"))
            .collect::<String>()
    };
    let slides = (0..slides)
        .map(|index| {
            let mut shapes = vec![text_shape(
                1,
                Some("title"),
                vec![format!("スライド {index} 売上と費用の見通し")],
            )];
            for body in 0..8 {
                let paragraphs = (0..12)
                    .map(|line| {
                        if variant && index % 7 == 0 && line == 3 {
                            format!("項目 {index}-{body}-{line} 改訂後の説明文")
                        } else {
                            format!("項目 {index}-{body}-{line} の説明文")
                        }
                    })
                    .collect();
                shapes.push(text_shape(2 + body, Some("body"), paragraphs));
            }
            let mut text = long(index);
            if variant && index % 5 == 0 {
                let middle = text
                    .char_indices()
                    .nth(text.chars().count() / 2)
                    .map_or(0, |(at, _)| at);
                text.insert_str(middle, "（追記）");
            }
            shapes.push(text_shape(20, None, vec![text]));
            let rows = (0..20)
                .map(|row| {
                    let mut cells = vec![format!("区分{row}"), format!("{}", row * 100)];
                    if variant && index % 3 == 0 {
                        cells.insert(1, format!("備考{row}"));
                    }
                    cells.push(format!("{}", row * 7));
                    cells
                })
                .collect();
            shapes.push(table_shape(30, rows));
            SlideIr {
                part: format!("ppt/slides/slide{}.xml", index + 1),
                index: index + 1,
                shapes,
                notes: Some(format!("ノート {index}")),
            }
        })
        .collect();
    PresentationIr {
        width: 9_144_000,
        height: 6_858_000,
        slides,
        media: Default::default(),
    }
}

fn main() {
    let slides: usize = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(400);
    let old = deck(slides, false);
    let mut new = deck(slides, true);
    // 途中に 1 枚差し込み、末尾を 1 枚落とす。
    let inserted = deck(1, true).slides.remove(0);
    new.slides.insert(slides / 2, inserted);
    new.slides.pop();
    for (index, slide) in new.slides.iter_mut().enumerate() {
        slide.index = index + 1;
    }

    let started = Instant::now();
    let diff = diff_presentations("old", "new", &old, &new, &DiffOptions::default());
    let elapsed = started.elapsed();
    println!("slides={slides} elapsed={elapsed:.2?}");
    println!("{:?}", diff.summary);
}
