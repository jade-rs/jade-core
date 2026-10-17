use futures::executor::block_on;
use jade_docx_layout::font_measurer::{
    FontdueLineWrapContext, FontdueMeasurer, RegisteredFontStyle,
};
use jade_docx_layout::text_extract::layout_docx_lines_from_bytes;
use std::fs;
use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../public/fixtures")
        .join(name)
}

fn bundled_font_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fonts/Noto_Sans_JP/static")
        .join(name)
}

fn bundled_noto_context() -> FontdueLineWrapContext {
    let mut measurer = FontdueMeasurer::new(12.0);
    measurer
        .register_font_path(
            "Noto Sans JP",
            400,
            RegisteredFontStyle::Normal,
            bundled_font_path("NotoSansJP-Regular.ttf"),
        )
        .expect("failed to load bundled Noto Sans JP regular");
    measurer
        .register_font_path(
            "Noto Sans JP",
            700,
            RegisteredFontStyle::Normal,
            bundled_font_path("NotoSansJP-Bold.ttf"),
        )
        .expect("failed to load bundled Noto Sans JP bold");
    FontdueLineWrapContext::new(measurer, None)
}

fn bundled_noto_context_with_yokohama_aliases() -> FontdueLineWrapContext {
    let mut measurer = FontdueMeasurer::new(12.0);
    for (family, weight, style, file) in [
        (
            "Noto Sans JP",
            400,
            RegisteredFontStyle::Normal,
            "NotoSansJP-Regular.ttf",
        ),
        (
            "Noto Sans JP",
            700,
            RegisteredFontStyle::Normal,
            "NotoSansJP-Bold.ttf",
        ),
        (
            "MS Gothic",
            400,
            RegisteredFontStyle::Normal,
            "NotoSansJP-Regular.ttf",
        ),
        (
            "ＭＳ ゴシック",
            400,
            RegisteredFontStyle::Normal,
            "NotoSansJP-Regular.ttf",
        ),
        (
            "MS Mincho",
            400,
            RegisteredFontStyle::Normal,
            "NotoSansJP-Regular.ttf",
        ),
        (
            "ＭＳ 明朝",
            400,
            RegisteredFontStyle::Normal,
            "NotoSansJP-Regular.ttf",
        ),
    ] {
        measurer
            .register_font_path(family, weight, style, bundled_font_path(file))
            .expect("failed to load bundled font alias");
    }
    FontdueLineWrapContext::new(measurer, None)
}

#[test]
fn noto_jp_yokohama_page1_first_line_tops_stay_stable() {
    let bytes = fs::read(fixture_path("noto-jp-yokohama_page6.docx"))
        .expect("failed to read noto-jp-yokohama_page6 fixture");
    let context = bundled_noto_context();
    let layout = block_on(layout_docx_lines_from_bytes(&bytes, &context))
        .expect("failed to layout noto-jp-yokohama_page6");

    let actual = layout
        .get_lines()
        .filter(|line| line.page == 1 && line.line_index_in_paragraph == 0)
        .take(26)
        .map(|line| {
            (
                line.paragraph_index + 1,
                line.text.clone(),
                (line.y * 1000.0).round() / 1000.0,
            )
        })
        .collect::<Vec<_>>();
    // The fixture starts with one blank paragraph, so the first visible content
    // begins at paragraph 2 in `jade-docx-layout`.
    let expected = vec![
        (2, "○横浜市市税条例".to_string(), 132.333),
        (3, "昭和25年8月11日".to_string(), 158.899),
        (4, "条例第34号".to_string(), 185.466),
        (
            5,
            "注　昭和60年3月から改正経過を注記した。".to_string(),
            212.032,
        ),
        (
            6,
            "市会の議決を経て、横浜市市税条例を次のように定める。".to_string(),
            238.598,
        ),
        (7, "横浜市市税条例".to_string(), 265.164),
        (8, "目次".to_string(), 291.730),
        (9, "第1章　総則(第1条―第20条)".to_string(), 318.296),
        (10, "第2章　普通税".to_string(), 344.862),
        (11, "第1節　市民税(第21条―第40条の10)".to_string(), 371.428),
        (12, "第2節　固定資産税(第41条―第70条)".to_string(), 397.995),
        (
            13,
            "第3節　軽自動車税(第71条―第81条の2)".to_string(),
            424.561,
        ),
        (
            14,
            "第4節　市たばこ税(第81条の3―第92条)".to_string(),
            451.127,
        ),
        (15, "第5節　削除".to_string(), 477.693),
        (
            16,
            "第6節　特別土地保有税(第104条の2―第120条)".to_string(),
            504.259,
        ),
        (17, "第3章　目的税".to_string(), 530.825),
        (18, "第1節　入湯税(第121条―第128条)".to_string(), 557.391),
        (
            19,
            "第2節　事業所税(第129条―第129条の13)".to_string(),
            583.957,
        ),
        (
            20,
            "第3節　都市計画税(第130条―第135条)".to_string(),
            610.524,
        ),
        (21, "附則".to_string(), 637.090),
        (22, "第1章　総則".to_string(), 663.656),
        (23, "(課税の根拠)".to_string(), 690.222),
        (
            24,
            "第1条　市税及びその賦課徴収については、地方税法(昭和25年法律第226号。以下".to_string(),
            716.788,
        ),
        (25, "(用語の意義)".to_string(), 796.486),
        (
            26,
            "第2条　この条例における次の各号に掲げる用語の意義は、それぞれ当該各号に定".to_string(),
            823.052,
        ),
        (
            27,
            "(1)　徴税吏員　市長又はその委任を受けた市職員をいう。".to_string(),
            876.185,
        ),
    ];

    assert_eq!(actual, expected);
}

#[test]
fn yokohama_page6_page3_reaches_item1_reason_paragraph() {
    let bytes =
        fs::read(fixture_path("yokohama_page6.docx")).expect("failed to read yokohama_page6");
    let context = bundled_noto_context_with_yokohama_aliases();
    let layout =
        block_on(layout_docx_lines_from_bytes(&bytes, &context)).expect("failed to layout fixture");

    let page_three = layout
        .get_lines()
        .filter(|line| line.page == 3)
        .collect::<Vec<_>>();
    assert!(
        page_three.iter().any(|line| {
            line.text
                .starts_with("(1)　徴収の猶予を受けた期間内に当該徴収の猶予を受けた金額")
        }),
        "page 3 should reach the item-1 reason paragraph instead of stopping at the earlier item-3 heading"
    );
}

#[test]
fn yokohama_page6_page5_reaches_kanka_reason_paragraph() {
    let bytes =
        fs::read(fixture_path("yokohama_page6.docx")).expect("failed to read yokohama_page6");
    let context = bundled_noto_context_with_yokohama_aliases();
    let layout =
        block_on(layout_docx_lines_from_bytes(&bytes, &context)).expect("failed to layout fixture");

    let page_five = layout
        .get_lines()
        .filter(|line| line.page == 5)
        .collect::<Vec<_>>();

    assert!(
        page_five.iter().any(|line| {
            line.text.starts_with(
                "3　法第15条の6の2第2項に規定する条例で定める事項は、次に掲げる事項とする",
            )
        }),
        "page 5 should include the article 15-6-2 item-3 heading",
    );
    assert!(
        page_five.iter().any(|line| {
            line.text.starts_with(
                "(1)　申請による換価の猶予を受けた期間内に当該申請による換価の猶予を受け",
            )
        }),
        "page 5 should reach the item-1 reason paragraph under article 15-6-2",
    );
    assert!(
        page_five.iter().any(|line| line
            .text
            .contains("た金額を納付し、又は納入することができないやむを得ない理由")),
        "page 5 should include the continuation of the item-1 reason paragraph",
    );
}
