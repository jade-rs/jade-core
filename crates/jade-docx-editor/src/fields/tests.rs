//! Field machinery end to end: documents built from XML, updated, and read
//! back from the saved package.

use super::*;
use crate::commands::CommandExecutor;
use crate::Options;
use std::io::Write as _;

const W: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";

fn package(body: &str, extra: &[(&str, &str)]) -> Vec<u8> {
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document {W}><w:body>{body}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/></w:sectPr></w:body></w:document>"
    );
    let mut parts: Vec<(String, String)> = vec![
        (
            "[Content_Types].xml".into(),
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
                "<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>",
                "<Default Extension=\"xml\" ContentType=\"application/xml\"/>",
                "<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>",
                "<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>",
                "<Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/>",
                "<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>",
                "<Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>",
                "<Override PartName=\"/word/header1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml\"/>",
                "</Types>"
            )
            .into(),
        ),
        (
            "_rels/.rels".into(),
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
                "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>",
                "<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>",
                "<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/>",
                "</Relationships>"
            )
            .into(),
        ),
        (
            "word/_rels/document.xml.rels".into(),
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
                "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>",
                "<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>",
                "</Relationships>"
            )
            .into(),
        ),
        (
            "word/styles.xml".into(),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:styles {W}><w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"a\"><w:name w:val=\"Normal\"/></w:style><w:style w:type=\"paragraph\" w:styleId=\"1\"><w:name w:val=\"heading 1\"/><w:basedOn w:val=\"a\"/><w:pPr><w:outlineLvl w:val=\"0\"/></w:pPr></w:style></w:styles>"
            ),
        ),
        (
            "word/settings.xml".into(),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:settings {W}><w:zoom w:percent=\"100\"/><w:docVars><w:docVar w:name=\"Client\" w:val=\"ACME\"/></w:docVars></w:settings>"
            ),
        ),
        (
            "docProps/core.xml".into(),
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">",
                "<dc:title>業務委託契約書</dc:title><dc:creator>山田 太郎</dc:creator><cp:lastModifiedBy>佐藤</cp:lastModifiedBy><cp:revision>4</cp:revision>",
                "<dcterms:created xsi:type=\"dcterms:W3CDTF\">2026-04-01T00:30:00Z</dcterms:created>",
                "</cp:coreProperties>"
            )
            .into(),
        ),
        (
            "docProps/app.xml".into(),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Company>Jade</Company><TotalTime>42</TotalTime></Properties>".into(),
        ),
        ("word/document.xml".into(), document),
    ];
    for (path, content) in extra {
        parts.push((path.to_string(), content.to_string()));
    }
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (path, content) in &parts {
            zip.start_file(path.as_str(), options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    cursor.into_inner()
}

fn load(bytes: &[u8]) -> WordDocument {
    block_on(WordDocument::load(bytes, &Options::default())).expect("load")
}

/// A complex field's XML.
fn field(code: &str, result: &str) -> String {
    format!(
        "<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> {} </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r>",
        code.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"),
        result
    )
}

fn paragraph(content: &str) -> String {
    format!("<w:p>{content}</w:p>")
}

/// The fields of the saved body as (code, result).
fn saved_fields(doc: &WordDocument) -> Vec<(String, String)> {
    let files = unpack(doc).unwrap();
    let body = parse_part(&files, "word/document.xml").unwrap();
    let story = scan::scan_document(body.root());
    story
        .fields
        .iter()
        .map(|field| {
            (
                field.display_code(&story.fields).trim().to_string(),
                field.result_text.clone(),
            )
        })
        .collect()
}

fn result_of(doc: &WordDocument, code_prefix: &str) -> String {
    saved_fields(doc)
        .into_iter()
        .find(|(code, _)| code.starts_with(code_prefix))
        .map(|(_, result)| result)
        .unwrap_or_else(|| panic!("no field {code_prefix}: {:?}", saved_fields(doc)))
}

fn environment() -> FieldEnvironment {
    FieldEnvironment {
        now: FieldDateTime {
            year: 2026,
            month: 10,
            day: 21,
            hour: 14,
            minute: 5,
            second: 9,
            weekday: 3,
        },
        utc_offset_seconds: 9 * 3600,
        user_name: "Jade User".into(),
        user_initials: "JU".into(),
        user_address: "Tokyo".into(),
        file_name: Some("contract.docx".into()),
        file_path: Some("/Users/jade/contract.docx".into()),
        file_size: Some(20480),
        page_count: Some(3),
        japanese: true,
        ..FieldEnvironment::default()
    }
}

#[test]
fn update_evaluates_each_kind_as_word_does() {
    let body = [
        paragraph(&field("AUTHOR", "old")),
        paragraph(&field("TITLE \\* Upper", "old")),
        paragraph(&field("DOCPROPERTY Company", "old")),
        paragraph(&field("DOCVARIABLE Client", "old")),
        paragraph(&field("DOCVARIABLE Missing", "old")),
        paragraph(&field("DATE \\@ \"ggge年M月d日\"", "old")),
        paragraph(&field("TIME \\@ \"H:mm\"", "old")),
        paragraph(&field("CREATEDATE \\@ \"yyyy/MM/dd HH:mm\"", "old")),
        paragraph(&field("USERNAME", "old")),
        paragraph(&field("USERINITIALS \\* Lower", "old")),
        paragraph(&field("FILENAME", "old")),
        paragraph(&field("FILENAME \\p", "old")),
        paragraph(&field("FILESIZE \\k", "old")),
        paragraph(&field("NUMPAGES \\* ROMAN", "old")),
        paragraph(&field("EDITTIME", "old")),
        paragraph(&field("REVNUM", "old")),
        paragraph(&field("LASTSAVEDBY", "old")),
        paragraph(&field("= 2 + 3 * 4", "old")),
        paragraph(&field("= 10 / 4 \\# \"0.00\"", "old")),
        paragraph(&field("= 1 / 0", "old")),
        paragraph(&field("QUOTE \"hello world\" \\* Caps", "old")),
        paragraph(&field("COMPARE 3 > 2", "old")),
        paragraph(&field("IF 3 > 2 \"yes\" \"no\"", "old")),
        paragraph(&field("SYMBOL 65", "old")),
        paragraph(&field("MERGEFIELD Name", "old")),
        paragraph(&field("SECTION", "old")),
        paragraph(&field("NUMWORDS", "old")),
        paragraph(&field("INFO Title", "old")),
        paragraph(&field("USERADDRESS", "old")),
        paragraph(&field("TEMPLATE", "old")),
        paragraph(&field("PRINTDATE", "old")),
        paragraph(&field("AUTOTEXTLIST \"pick one\" \\s Normal", "old")),
        paragraph(&field("Unknown", "old")),
    ]
    .concat();
    let mut doc = load(&package(&body, &[]));
    let written = update_fields(&mut doc, FieldScope::Document, &environment()).unwrap();
    assert!(written > 20, "{written}");
    let results = saved_fields(&doc);
    let expect = [
        ("AUTHOR", "山田 太郎"),
        ("TITLE", "業務委託契約書"),
        ("DOCPROPERTY Company", "Jade"),
        ("DOCVARIABLE Client", "ACME"),
        (
            "DOCVARIABLE Missing",
            "エラー! 文書変数が指定されていません。",
        ),
        ("DATE", "令和8年10月21日"),
        ("TIME", "14:05"),
        // 00:30 UTC is 09:30 in Tokyo.
        ("CREATEDATE", "2026/04/01 09:30"),
        ("USERNAME", "Jade User"),
        ("USERINITIALS", "ju"),
        ("FILENAME \\p", "/Users/jade/contract.docx"),
        ("FILENAME", "contract.docx"),
        ("FILESIZE", "20"),
        ("NUMPAGES", "III"),
        ("EDITTIME", "42"),
        ("REVNUM", "4"),
        ("LASTSAVEDBY", "佐藤"),
        ("= 2 + 3 * 4", "14"),
        ("= 10 / 4", "2.50"),
        ("= 1 / 0", "!0 で除算しました"),
        ("QUOTE", "Hello World"),
        ("COMPARE", "1"),
        ("IF", "yes"),
        ("SYMBOL", "A"),
        ("MERGEFIELD", "«Name»"),
        ("SECTION", "1"),
        ("INFO", "業務委託契約書"),
        ("USERADDRESS", "Tokyo"),
        ("TEMPLATE", "Normal.dotm"),
        ("PRINTDATE", "0000/00/00 0:00:00"),
        ("AUTOTEXTLIST", "pick one"),
        ("Unknown", "エラー! ブックマークが定義されていません。"),
    ];
    for (prefix, expected) in expect {
        let found = results
            .iter()
            .find(|(code, _)| {
                code.starts_with(prefix) && (prefix != "FILENAME" || !code.contains("\\p"))
            })
            .unwrap_or_else(|| panic!("no {prefix} in {results:?}"));
        assert_eq!(found.1, expected, "{prefix}");
    }
    let words = result_of(&doc, "NUMWORDS");
    assert!(words.parse::<usize>().is_ok(), "{words}");
}

#[test]
fn sequences_lists_styles_and_references_count_in_document_order() {
    let body = [
        format!(
            "<w:p><w:pPr><w:pStyle w:val=\"1\"/></w:pPr><w:r><w:t>第一章</w:t></w:r></w:p>"
        ),
        paragraph(&format!(
            "<w:r><w:t xml:space=\"preserve\">図 </w:t></w:r>{}",
            field("SEQ Figure \\* ARABIC", "9")
        )),
        paragraph(&format!(
            "<w:bookmarkStart w:id=\"1\" w:name=\"Second\"/>{}<w:bookmarkEnd w:id=\"1\"/>",
            field("SEQ Figure", "9")
        )),
        paragraph(&field("SEQ Figure \\c \\* roman", "9")),
        paragraph(&field("SEQ Figure \\r 10", "9")),
        paragraph(&field("SEQ Figure", "9")),
        paragraph(&field("SEQ Figure Second", "9")),
        paragraph(&field("SEQ Table \\h", "9")),
        paragraph(&format!(
            "{}{}",
            field("LISTNUM NumberDefault", "x"),
            field("LISTNUM", "x")
        )),
        paragraph(&field("LISTNUM LegalDefault \\l 2", "x")),
        paragraph(&field("STYLEREF \"見出し 1\"", "x")),
        paragraph(&field("STYLEREF 1", "x")),
        paragraph(&format!(
            "<w:bookmarkStart w:id=\"2\" w:name=\"Price\"/><w:r><w:t>1,200</w:t></w:r><w:bookmarkEnd w:id=\"2\"/>"
        )),
        paragraph(&field("REF Price", "x")),
        paragraph(&field("REF Price \\p", "x")),
        paragraph(&field("Price", "x")),
        paragraph(&field("= Price * 2 \\# \"#,##0\"", "x")),
        paragraph(&field("REF Nowhere", "x")),
        paragraph(&field("SET Greeting \"こんにちは\"", "")),
        paragraph(&field("REF Greeting", "x")),
        paragraph(&field("IF Greeting = \"こん*\" \"match\" \"none\"", "x")),
        paragraph(&field("AUTONUMLGL", "x")),
        paragraph(&field("AUTONUM", "x")),
    ]
    .concat();
    let mut doc = load(&package(&body, &[]));
    update_fields(&mut doc, FieldScope::Document, &environment()).unwrap();
    let results: Vec<String> = saved_fields(&doc).into_iter().map(|(_, r)| r).collect();
    assert_eq!(
        results,
        vec![
            "1",
            "2",
            "ii",
            "10",
            "11",
            "2",
            "",
            "1)",
            "a)",
            "1.1.",
            "第一章",
            "第一章",
            "1,200",
            "上",
            "1,200",
            "2,400",
            "エラー! 参照元が見つかりません。",
            "こんにちは",
            "こんにちは",
            "match",
            "1.",
            "2."
        ]
    );
}

#[test]
fn nested_fields_update_inside_out_and_tables_sum() {
    let table = concat!(
        "<w:tbl><w:tblGrid><w:gridCol/><w:gridCol/></w:tblGrid>",
        "<w:tr><w:tc><w:p><w:r><w:t>Item</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Price</w:t></w:r></w:p></w:tc></w:tr>",
        "<w:tr><w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>100</w:t></w:r></w:p></w:tc></w:tr>",
        "<w:tr><w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>250</w:t></w:r></w:p></w:tc></w:tr>",
        "<w:tr><w:tc><w:p><w:r><w:t>Total</w:t></w:r></w:p></w:tc><w:tc><w:p>"
    )
    .to_string()
        + &field("=SUM(ABOVE) \\# \"¥#,##0\"", "0")
        + "</w:p></w:tc></w:tr></w:tbl>";
    let nested = format!(
        "<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> IF </w:instrText></w:r>{}<w:r><w:instrText xml:space=\"preserve\"> = 3 \"three pages\" \"other\" </w:instrText></w:r><w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>stale</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r>",
        field("NUMPAGES", "1")
    );
    let body = format!("{table}{}", paragraph(&nested));
    let mut doc = load(&package(&body, &[]));
    update_fields(&mut doc, FieldScope::Document, &environment()).unwrap();
    // `#` with no digit reads as a space, as Word documents (`$###` → `$ 15`).
    assert_eq!(result_of(&doc, "=SUM(ABOVE)"), "¥  350");
    let fields = saved_fields(&doc);
    let outer = fields
        .iter()
        .find(|(code, _)| code.starts_with("IF"))
        .unwrap();
    assert_eq!(outer.0, "IF { NUMPAGES } = 3 \"three pages\" \"other\"");
    assert_eq!(outer.1, "three pages");
    // The nested result is written too.
    assert_eq!(result_of(&doc, "NUMPAGES"), "3");
}

#[test]
fn selection_updates_only_the_fields_it_touches() {
    let body = [
        paragraph(&format!(
            "<w:r><w:t xml:space=\"preserve\">A </w:t></w:r>{}",
            field("AUTHOR", "old")
        )),
        paragraph(&field("TITLE", "old")),
    ]
    .concat();
    let mut doc = load(&package(&body, &[]));
    let caret = SelectionRange::collapsed(TextPosition::new(0, 2));
    update_fields(&mut doc, FieldScope::Selection(caret), &environment()).unwrap();
    assert_eq!(result_of(&doc, "AUTHOR"), "山田 太郎");
    assert_eq!(result_of(&doc, "TITLE"), "old");
    let infos = fields_at(&doc, caret);
    assert_eq!(infos.len(), 1);
    assert_eq!(infos[0].name, "AUTHOR");
}

#[test]
fn locked_fields_keep_their_results_until_unlocked_and_unlink_leaves_text() {
    let body = paragraph(&field("AUTHOR", "old"));
    let mut doc = load(&package(&body, &[]));
    assert_eq!(
        set_fields_locked(&mut doc, FieldScope::Document, true).unwrap(),
        1
    );
    update_fields(&mut doc, FieldScope::Document, &environment()).unwrap();
    assert_eq!(result_of(&doc, "AUTHOR"), "old");
    assert!(fields_at(&doc, SelectionRange::collapsed(TextPosition::new(0, 0)))[0].locked);
    set_fields_locked(&mut doc, FieldScope::Document, false).unwrap();
    update_fields(&mut doc, FieldScope::Document, &environment()).unwrap();
    assert_eq!(result_of(&doc, "AUTHOR"), "山田 太郎");
    unlink_fields(&mut doc, FieldScope::Document).unwrap();
    assert!(saved_fields(&doc).is_empty());
    let text: String = doc
        .paragraph(0)
        .unwrap()
        .runs
        .iter()
        .map(|run| run.text.as_str())
        .collect();
    assert_eq!(text, "山田 太郎");
}

#[test]
fn inserts_and_edits_fields_from_their_codes() {
    let body = paragraph("<w:r><w:t xml:space=\"preserve\">Date: </w:t></w:r>");
    let mut doc = load(&package(&body, &[]));
    let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 6)));
    insert_field(
        &mut doc,
        &mut selection,
        "DATE \\@ \"yyyy年M月d日\"",
        &environment(),
    )
    .unwrap();
    assert_eq!(result_of(&doc, "DATE"), "2026年10月21日");
    assert_eq!(selection.unwrap().focus, TextPosition::new(0, 6));
    // The saved code is the one typed.
    assert_eq!(saved_fields(&doc)[0].0, "DATE \\@ \"yyyy年M月d日\"");

    let mut selection = Some(SelectionRange::collapsed(TextPosition::new(0, 6)));
    insert_field(
        &mut doc,
        &mut selection,
        "IF { NUMPAGES } > 2 \"long\" \"short\"",
        &environment(),
    )
    .unwrap();
    assert_eq!(result_of(&doc, "IF"), "long");

    let caret = SelectionRange::collapsed(TextPosition::new(0, 6));
    let before = fields_at(&doc, caret);
    assert!(!before.is_empty());
    edit_field_code(&mut doc, caret, "USERNAME \\* Upper", &environment()).unwrap();
    let codes: Vec<String> = saved_fields(&doc)
        .into_iter()
        .map(|(code, _)| code)
        .collect();
    assert!(
        codes.iter().any(|code| code == "USERNAME \\* Upper"),
        "{codes:?}"
    );
    assert_eq!(result_of(&doc, "USERNAME"), "JADE USER");
}

#[test]
fn prompts_are_listed_and_answers_fill_ask_and_fillin() {
    let body = [
        paragraph(&field("ASK Name \"お名前は?\" \\d \"匿名\"", "")),
        paragraph(&field("REF Name", "x")),
        paragraph(&field("FILLIN \"部署は?\" \\d \"総務\"", "")),
    ]
    .concat();
    let mut doc = load(&package(&body, &[]));
    let prompts = field_prompts(&doc, FieldScope::Document);
    assert_eq!(prompts.len(), 2);
    assert_eq!(prompts[0].field, "ASK");
    assert_eq!(prompts[0].prompt, "お名前は?");
    assert_eq!(prompts[0].default, "匿名");
    assert_eq!(prompts[0].bookmark.as_deref(), Some("Name"));
    assert_eq!(prompts[1].prompt, "部署は?");
    let mut env = environment();
    env.prompt_answers
        .insert(prompts[0].key.clone(), "鈴木".into());
    env.prompt_answers
        .insert(prompts[1].key.clone(), "開発".into());
    update_fields(&mut doc, FieldScope::Document, &env).unwrap();
    assert_eq!(result_of(&doc, "REF Name"), "鈴木");
    assert_eq!(result_of(&doc, "FILLIN"), "開発");
}

#[test]
fn property_fields_read_and_write_document_properties() {
    let body = [
        paragraph(&field("AUTHOR \"Hanako\"", "old")),
        paragraph(&field("DOCPROPERTY \"Project Code\"", "old")),
        paragraph(&field("DOCPROPERTY Budget \\# \"#,##0\"", "old")),
    ]
    .concat();
    let mut doc = load(&package(&body, &[]));
    let mut properties = document_properties(&doc);
    assert_eq!(properties.title, "業務委託契約書");
    properties.custom.push(CustomProperty {
        name: "Project Code".into(),
        value: CustomValue::Text("JX-7".into()),
    });
    properties.custom.push(CustomProperty {
        name: "Budget".into(),
        value: CustomValue::Number(1500000.0),
    });
    set_document_properties(&mut doc, &properties).unwrap();
    update_fields(&mut doc, FieldScope::Document, &environment()).unwrap();
    assert_eq!(result_of(&doc, "AUTHOR"), "Hanako");
    assert_eq!(document_properties(&doc).author, "Hanako");
    assert_eq!(result_of(&doc, "DOCPROPERTY \"Project Code\""), "JX-7");
    assert_eq!(result_of(&doc, "DOCPROPERTY Budget"), "1,500,000");

    set_document_variables(&mut doc, &[("Client".into(), "Jade".into())]).unwrap();
    assert_eq!(
        document_variables(&doc),
        vec![("Client".to_string(), "Jade".to_string())]
    );
}

#[test]
fn headers_update_with_the_document_and_include_text_reads_local_files() {
    let header = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:hdr {W}><w:p>{}</w:p></w:hdr>",
        field("STYLEREF 1", "old")
    );
    let body = [
        format!("<w:p><w:pPr><w:pStyle w:val=\"1\"/></w:pPr><w:r><w:t>概要</w:t></w:r></w:p>"),
        paragraph(&field("INCLUDETEXT \"notes.txt\"", "cached")),
        paragraph(&field("INCLUDETEXT \"missing.txt\"", "cached")),
        paragraph(&field("INCLUDEPICTURE \"logo.png\" \\d", "")),
    ]
    .concat();
    let mut doc = load(&package(&body, &[("word/header1.xml", &header)]));
    let mut env = environment();
    env.read_file = Some(Box::new(|path: &str| match path {
        "notes.txt" => Some("included line".as_bytes().to_vec()),
        "logo.png" => {
            let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
            png.extend_from_slice(&64u32.to_be_bytes());
            png.extend_from_slice(&32u32.to_be_bytes());
            Some(png)
        }
        _ => None,
    }));
    update_fields(&mut doc, FieldScope::Document, &env).unwrap();
    assert_eq!(
        result_of(&doc, "INCLUDETEXT \"notes.txt\""),
        "included line"
    );
    let files = unpack(&doc).unwrap();
    let body = String::from_utf8(files["word/document.xml"].clone()).unwrap();
    assert!(body.contains("r:embed="), "the picture is embedded");
    assert!(files
        .keys()
        .any(|path| path.starts_with("word/media/field")));
    assert_eq!(result_of(&doc, "INCLUDETEXT \"missing.txt\""), "cached");
    let files = unpack(&doc).unwrap();
    let header = String::from_utf8(files["word/header1.xml"].clone()).unwrap();
    assert!(header.contains(">概要<"), "{header}");
}

#[test]
fn field_commands_undo_and_redo_whole() {
    let body = paragraph(&field("AUTHOR", "old"));
    let mut doc = load(&package(&body, &[]));
    let mut selection = None;
    let mut executor = CommandExecutor::new();
    {
        let mut ctx = CommandContext::new(&mut doc, &mut selection);
        executor
            .execute(
                Box::new(FieldCommand::update(FieldScope::Document, environment())),
                &mut ctx,
            )
            .unwrap();
    }
    assert_eq!(result_of(&doc, "AUTHOR"), "山田 太郎");
    {
        let mut ctx = CommandContext::new(&mut doc, &mut selection);
        assert!(executor.undo(&mut ctx).unwrap());
    }
    assert_eq!(result_of(&doc, "AUTHOR"), "old");
    {
        let mut ctx = CommandContext::new(&mut doc, &mut selection);
        assert!(executor.redo(&mut ctx).unwrap());
    }
    assert_eq!(result_of(&doc, "AUTHOR"), "山田 太郎");
}

#[test]
fn code_display_shows_each_field_once_in_braces() {
    let body = paragraph(&format!(
        "<w:r><w:t xml:space=\"preserve\">p. </w:t></w:r>{}",
        field("PAGE \\* MERGEFORMAT", "3")
    ));
    let doc = load(&package(&body, &[]));
    let all = field_code_display(
        &doc,
        &FieldCodeView {
            all: true,
            toggled: HashSet::new(),
        },
    );
    let shown: Vec<&String> = all.values().filter(|text| !text.is_empty()).collect();
    assert_eq!(shown, vec!["{ PAGE \\* MERGEFORMAT }"]);
    // ⇧F9 on the field turns it back.
    let ids = field_toggle_ids(&doc, SelectionRange::collapsed(TextPosition::new(0, 3)));
    assert_eq!(ids.len(), 1);
    let toggled = field_code_display(
        &doc,
        &FieldCodeView {
            all: true,
            toggled: ids.iter().copied().collect(),
        },
    );
    assert!(toggled.is_empty());
    let one = field_code_display(
        &doc,
        &FieldCodeView {
            all: false,
            toggled: ids.into_iter().collect(),
        },
    );
    assert_eq!(one.len(), all.len());
}
