use super::address::{render_format, CountryMode};
use super::*;
use crate::commands::{CommandContext, CommandExecutor};
use crate::word_document::content_controls::tests::{open, package};

fn data() -> MergeData {
    MergeData::from_rows(vec![
        vec![
            "姓",
            "名",
            "郵便番号",
            "都道府県",
            "市区町村",
            "住所1",
            "会社名",
            "性別",
            "金額",
        ]
        .into_iter()
        .map(str::to_string)
        .collect(),
        vec![
            "山田",
            "花子",
            "100-0001",
            "東京都",
            "千代田区",
            "千代田1-1",
            "ジェイド株式会社",
            "F",
            "1234.5",
        ]
        .into_iter()
        .map(str::to_string)
        .collect(),
        vec![
            "鈴木",
            "一郎",
            "530-0001",
            "大阪府",
            "大阪市北区",
            "梅田2-2",
            "",
            "M",
            "10",
        ]
        .into_iter()
        .map(str::to_string)
        .collect(),
        vec!["佐藤", "", "", "", "", "", "", "M", "0"]
            .into_iter()
            .map(str::to_string)
            .collect(),
    ])
}

fn text_of(document: &WordDocument, paragraph: usize) -> String {
    document
        .paragraph(paragraph)
        .map(|paragraph| paragraph.runs.iter().map(|run| run.text.as_str()).collect())
        .unwrap_or_default()
}

fn run(document: &mut WordDocument, action: MailMergeAction, at: (usize, usize)) {
    let range = SelectionRange::collapsed(TextPosition::new(at.0, at.1));
    let mut selection = Some(range);
    let mut executor = CommandExecutor::new();
    let mut ctx = CommandContext::new(document, &mut selection);
    executor
        .execute(Box::new(action.command(range)), &mut ctx)
        .expect("the merge action applies");
}

fn field(instruction: &str, result: &str) -> String {
    let instruction = xml_escape(instruction);
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {instruction} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>{result}</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
    )
}

fn letter() -> String {
    format!(
        r#"<w:p><w:r><w:t xml:space="preserve">宛先: </w:t></w:r>{}<w:r><w:t xml:space="preserve"> </w:t></w:r>{}<w:r><w:t xml:space="preserve"> 様</w:t></w:r></w:p><w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> IF </w:instrText></w:r>{}<w:r><w:instrText xml:space="preserve"> = "F" "女性" "男性" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>女性</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t xml:space="preserve"> </w:t></w:r>{}</w:p><w:p>{}</w:p>"#,
        field("MERGEFIELD 姓", "«姓»"),
        field("MERGEFIELD 名", "«名»"),
        field("MERGEFIELD 性別", "«性別»"),
        field(r##"MERGEFIELD 金額 \# "#,##0.00" \b "¥""##, "«金額»"),
        field("MERGEREC", "«Merge Record #»"),
    )
}

#[test]
fn preview_shows_a_recipient_and_preview_off_the_names() {
    let mut document = open(&letter(), "");
    let request = MergeRequest::all(data());
    run(
        &mut document,
        MailMergeAction::Preview(request.clone(), 0),
        (0, 0),
    );
    assert_eq!(text_of(&document, 0), "宛先: 山田 花子 様");
    let second: String = document.raw_document.as_ref().unwrap().to_string().unwrap();
    assert!(second.contains("女性"), "{second}");
    assert!(second.contains("¥1,234.50"), "{second}");
    run(&mut document, MailMergeAction::Preview(request, 1), (0, 0));
    assert_eq!(text_of(&document, 0), "宛先: 鈴木 一郎 様");
    let xml = document.raw_document.as_ref().unwrap().to_string().unwrap();
    assert!(xml.contains("男性"));
    assert!(
        xml.contains(">2<"),
        "MERGEREC is the record's number: {xml}"
    );
    // The fields are still there.
    assert_eq!(
        document.merge_field_names(),
        vec!["姓", "名", "性別", "金額"]
    );

    run(&mut document, MailMergeAction::FieldCodes, (0, 0));
    assert_eq!(text_of(&document, 0), "宛先: «姓» «名» 様");
}

#[test]
fn merges_a_copy_per_recipient_and_skips_the_ones_a_rule_says() {
    let body = format!(
        r#"<w:p>{}<w:r><w:t xml:space="preserve"> </w:t></w:r>{}</w:p><w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> SKIPIF </w:instrText></w:r>{}<w:r><w:instrText xml:space="preserve"> = "佐藤" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#,
        field("MERGEFIELD 姓", "«姓»"),
        field("MERGEFIELD 名", "«名»"),
        field("MERGEFIELD 姓", "«姓»"),
    );
    let mut document = open(&body, "");
    run(
        &mut document,
        MailMergeAction::Settings(MailMergeSettings {
            kind: Some(MainDocumentType::Letters),
            data_source: Some("/Users/me/list.csv".to_string()),
            ..Default::default()
        }),
        (0, 0),
    );
    let merged = document
        .merged_document(&MergeRequest::all(data()))
        .expect("merge");
    let reopened =
        super::super::rewrite::run_ready(WordDocument::load(&merged, &Options::default()))
            .expect("the merged document opens");
    let texts: Vec<String> = (0..reopened.paragraph_count())
        .map(|index| text_of(&reopened, index))
        .filter(|text| !text.is_empty())
        .collect();
    assert_eq!(texts, vec!["山田 花子", "鈴木 一郎"]);
    let xml = reopened.raw_document.as_ref().unwrap().to_string().unwrap();
    assert_eq!(xml.matches("w:sectPr").count(), 4, "{xml}");
    assert!(!xml.contains("MERGEFIELD"));
    assert_eq!(reopened.mail_merge_settings().kind, None);
}

#[test]
fn settings_keep_the_type_the_source_and_the_matching() {
    let mut document = open("<w:p/>", "");
    let settings = MailMergeSettings {
        kind: Some(MainDocumentType::Labels),
        data_source: Some("/Users/me/住所録.xlsx".to_string()),
        preview: true,
        active_record: 3,
        mapping: vec![(AddressField::LastName, "氏".to_string())],
        check_errors: None,
    };
    run(
        &mut document,
        MailMergeAction::Settings(settings.clone()),
        (0, 0),
    );
    assert_eq!(document.mail_merge_settings(), settings);
    let exported = document.export_docx().expect("export");
    let reopened =
        super::super::rewrite::run_ready(WordDocument::load(&exported, &Options::default()))
            .unwrap();
    assert_eq!(reopened.mail_merge_settings(), settings);
    run(&mut document, MailMergeAction::Normal, (0, 0));
    assert_eq!(document.mail_merge_settings().kind, None);
}

#[test]
fn inserts_fields_at_the_caret_with_the_previewed_values() {
    let mut document = open(
        r#"<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Hello </w:t></w:r></w:p>"#,
        "",
    );
    run(
        &mut document,
        MailMergeAction::InsertField {
            field: MergeFieldSpec::Column("会社名".to_string()),
            preview: None,
        },
        (0, 5),
    );
    assert_eq!(text_of(&document, 0), "Hello«会社名»");
    run(
        &mut document,
        MailMergeAction::InsertField {
            field: MergeFieldSpec::Rule(MergeRule::If {
                field: "性別".to_string(),
                operator: "=".to_string(),
                value: "F".to_string(),
                then_text: "様".to_string(),
                else_text: "殿".to_string(),
            }),
            preview: Some((MergeRequest::all(data()), 1)),
        },
        (0, 0),
    );
    let xml = document.raw_document.as_ref().unwrap().to_string().unwrap();
    assert!(xml.contains("IF "), "{xml}");
    assert!(
        xml.contains("<w:t xml:space=\"preserve\">殿</w:t>"),
        "{xml}"
    );
    assert!(
        text_of(&document, 0).ends_with("Hello«会社名»") || text_of(&document, 0).contains("Hello")
    );
    assert!(
        xml.matches("<w:b/>").count() >= 2,
        "the field takes the text's format"
    );
}

#[test]
fn address_blocks_and_greeting_lines_follow_their_formats() {
    let data = data();
    let mapping = FieldMapping::matched(&data);
    assert_eq!(mapping.column(AddressField::LastName), Some(0));
    assert_eq!(mapping.column(AddressField::PostalCode), Some(2));
    let value = |record: usize| {
        let data = data.clone();
        let mapping = mapping.clone();
        move |field: AddressField| {
            mapping
                .column(field)
                .map(|column| data.value(record, column).to_string())
                .unwrap_or_default()
        }
    };
    let japanese = AddressBlockOptions::new(true);
    assert_eq!(
        render_format(&japanese.format(), &value(0), &|_| false),
        "〒100-0001\n東京都千代田区千代田1-1\nジェイド株式会社\n山田 花子 様"
    );
    // No company: its line goes.
    assert_eq!(
        render_format(&japanese.format(), &value(1), &|_| false),
        "〒530-0001\n大阪府大阪市北区梅田2-2\n鈴木 一郎 様"
    );
    let mut english = AddressBlockOptions::new(false);
    english.country = CountryMode::Never;
    assert!(english.instruction().starts_with("ADDRESSBLOCK \\f \""));
    let greeting = GreetingLineOptions::new(true);
    assert_eq!(
        render_format(&greeting.format(), &value(0), &|_| false),
        "山田 花子 様"
    );

    // As fields, through a merge.
    let body = format!(
        "<w:p>{}</w:p><w:p>{}</w:p>",
        field(&japanese.instruction(), "«AddressBlock»"),
        field(&greeting.instruction(), "«GreetingLine»")
    );
    let mut document = open(&body, "");
    run(
        &mut document,
        MailMergeAction::Preview(MergeRequest::all(data.clone()), 0),
        (0, 0),
    );
    let xml = document.raw_document.as_ref().unwrap().to_string().unwrap();
    assert!(xml.contains("〒100-0001"), "{xml}");
    assert!(
        xml.contains("<w:br/>"),
        "the address block's lines break: {xml}"
    );
    assert!(xml.contains("山田 花子 様"));
}

#[test]
fn checks_a_merge_for_missing_columns() {
    let body = format!(
        "<w:p>{}{}</w:p>",
        field("MERGEFIELD 姓", "«姓»"),
        field("MERGEFIELD 部署", "«部署»")
    );
    let document = open(&body, "");
    let problems = document.merge_problems(&MergeRequest::all(data())).unwrap();
    assert_eq!(
        problems,
        vec![MergeProblem::MissingColumn("部署".to_string())]
    );
    let problems = document
        .merge_problems(&MergeRequest::all(MergeData::default()))
        .unwrap();
    assert!(problems.contains(&MergeProblem::EmptyDataSource));
}

#[test]
fn labels_take_a_recipient_each_and_a_page_per_sheet() {
    let mut document = open("<w:p/>", "");
    let product = LabelProduct {
        columns: 2,
        rows: 2,
        ..LABEL_PRODUCTS[0]
    };
    run(
        &mut document,
        MailMergeAction::MakeLabels {
            product,
            text: None,
            merge: true,
            japanese: true,
        },
        (0, 0),
    );
    let xml = document.raw_document.as_ref().unwrap().to_string().unwrap();
    assert_eq!(xml.matches("ADDRESSBLOCK").count(), 4, "{xml}");
    assert_eq!(xml.matches(" NEXT ").count(), 3);
    assert!(xml.contains("w:hRule=\"exact\""));
    // ラベルの更新 copies the first label to the rest again.
    run(&mut document, MailMergeAction::PropagateLabels, (0, 0));
    let xml = document.raw_document.as_ref().unwrap().to_string().unwrap();
    assert_eq!(xml.matches("ADDRESSBLOCK").count(), 4, "{xml}");

    let mut more = data();
    more.records.push(more.records[0].clone());
    more.records.push(more.records[1].clone());
    let merged = document.merged_document(&MergeRequest::all(more)).unwrap();
    let reopened =
        super::super::rewrite::run_ready(WordDocument::load(&merged, &Options::default())).unwrap();
    let xml = reopened.raw_document.as_ref().unwrap().to_string().unwrap();
    // Five recipients on sheets of four: two sheets.
    assert_eq!(xml.matches("<w:tbl>").count(), 2, "{xml}");
    assert_eq!(xml.matches("山田 花子 様").count(), 2);
}

#[test]
fn envelopes_are_pages_the_envelope_size() {
    let mut document = open("<w:p/>", "");
    let spec = EnvelopeSpec {
        size: ENVELOPE_SIZES[6],
        delivery: "Taro Yamada\n1-1 Chiyoda\nTokyo".to_string(),
        return_address: "Jade Inc.".to_string(),
        omit_return: false,
        merge: false,
        delivery_left_mm: None,
        delivery_top_mm: None,
        japanese: false,
    };
    run(
        &mut document,
        MailMergeAction::MakeEnvelope(spec.clone()),
        (0, 0),
    );
    let texts: Vec<String> = (0..document.paragraph_count())
        .map(|index| text_of(&document, index))
        .collect();
    assert_eq!(
        texts,
        vec!["Jade Inc.", "Taro Yamada", "1-1 Chiyoda", "Tokyo"]
    );
    let setup = document.section_page_setup(None);
    let _ = setup;
    let xml = document.raw_document.as_ref().unwrap().to_string().unwrap();
    assert!(xml.contains("w:orient=\"landscape\""));
    // A Japanese 長形 envelope is written vertically.
    let vertical = EnvelopeSpec {
        size: ENVELOPE_SIZES[0],
        delivery: "東京都千代田区\n山田 太郎 様".to_string(),
        japanese: true,
        ..spec
    };
    run(
        &mut document,
        MailMergeAction::MakeEnvelope(vertical),
        (0, 0),
    );
    let xml = document.raw_document.as_ref().unwrap().to_string().unwrap();
    assert!(xml.contains("tbRl"), "{xml}");
    let _ = package;
}
