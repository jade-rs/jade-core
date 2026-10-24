//! docx を組み立てて比較する結合テスト。
//!
//! フィクスチャをファイルで置かず、その場で最小の docx を作る。paraId を
//! 1 件ずつ指定したいので、Word や python-docx に書かせるより確実で、
//! 何を試しているのかもテストを読めば分かる。

use std::io::{Cursor, Write};

use jade_docx_diff::{ChangeKind, DiffOptions, DiffSummary, Limits, load_document_bytes};

/// 段落 1 つぶんの XML。`para_id` を渡すと `w14:paraId` を振る。
fn para(text: &str, para_id: Option<&str>) -> String {
    let id = match para_id {
        Some(id) => format!(r#" w14:paraId="{id}""#),
        None => String::new(),
    };
    format!(r#"<w:p{id}><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#)
}

/// 表 1 つぶんの XML。行ごとにセルの本文を渡す。
fn table(rows: &[&[&str]]) -> String {
    let mut out = String::from("<w:tbl>");
    for row in rows {
        out.push_str("<w:tr>");
        for cell in *row {
            out.push_str("<w:tc>");
            out.push_str(&para(cell, None));
            out.push_str("</w:tc>");
        }
        out.push_str("</w:tr>");
    }
    out.push_str("</w:tbl>");
    out
}

/// 本文の XML から最小の docx (zip) を作る。
fn docx(body: &str) -> Vec<u8> {
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document
  xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"
  xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"
  mc:Ignorable="w14"
  xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006">
  <w:body>{body}</w:body>
</w:document>"#
    );

    let mut buffer = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buffer));
        let options: zip::write::SimpleFileOptions =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        zip.start_file("[Content_Types].xml", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#,
        )
        .unwrap();

        zip.start_file("_rels/.rels", options).unwrap();
        zip.write_all(
            br#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#,
        )
        .unwrap();

        zip.start_file("word/document.xml", options).unwrap();
        zip.write_all(document.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    buffer
}

fn compare(old_body: &str, new_body: &str) -> jade_docx_diff::DocumentDiff {
    let limits = Limits::default();
    let old = load_document_bytes(&docx(old_body), &limits).expect("旧の読み込み");
    let new = load_document_bytes(&docx(new_body), &limits).expect("新の読み込み");
    jade_docx_diff::diff_documents("old.docx", "new.docx", &old, &new, &DiffOptions::default())
}

/// 0 でない項目だけを並べる。何が報告されたのかを一目で見るため。
fn reported(summary: &DiffSummary) -> Vec<(&'static str, usize)> {
    [
        ("paragraphs_added", summary.paragraphs_added),
        ("paragraphs_removed", summary.paragraphs_removed),
        ("paragraphs_changed", summary.paragraphs_changed),
        ("paragraphs_moved", summary.paragraphs_moved),
        ("tables_added", summary.tables_added),
        ("tables_removed", summary.tables_removed),
        ("table_rows_added", summary.table_rows_added),
        ("table_rows_removed", summary.table_rows_removed),
        ("table_rows_changed", summary.table_rows_changed),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .collect()
}

fn chapters(count: usize) -> String {
    (1..=count)
        .map(|n| {
            para(
                &format!("第{n}条 この条項の内容は {n} です。"),
                Some(&format!("A00000{n:02}")),
            )
        })
        .collect()
}

#[test]
fn identical_documents_have_no_changes() {
    let diff = compare(&chapters(6), &chapters(6));
    assert!(!diff.has_changes(), "{:?}", reported(&diff.summary));
}

#[test]
fn editing_one_paragraph_reports_only_that_paragraph() {
    let old = chapters(6);
    let new = old.replace(
        "第3条 この条項の内容は 3 です。",
        "第3条 この条項の内容は 3 に改めました。",
    );

    let diff = compare(&old, &new);
    assert_eq!(reported(&diff.summary), vec![("paragraphs_changed", 1)]);
}

#[test]
fn a_rewritten_paragraph_still_matches_through_its_para_id() {
    // 文面を丸ごと差し替えても、Word 上は同じ段落。paraId を最優先にする
    // 値がここに出る。文字の似ぐあいだけで見ると削除 + 追加に割れてしまう。
    let old = para("第3条 委託料は月額 500,000 円とする。", Some("B0000001"));
    let new = para(
        "第3条 甲は乙に対し、別紙に定める方法により報酬を支払う。",
        Some("B0000001"),
    );

    let diff = compare(&old, &new);
    assert_eq!(reported(&diff.summary), vec![("paragraphs_changed", 1)]);

    let jade_docx_diff::BlockChange::Paragraph(change) = &diff.blocks[0] else {
        panic!("段落のはず: {:?}", diff.blocks[0]);
    };
    assert!(
        change.matched_by_para_id,
        "paraId で対応が付いたと分かること"
    );
}

#[test]
fn inserting_a_paragraph_does_not_shift_the_rest() {
    let old = chapters(6);
    let mut new = String::new();
    for (index, chapter) in (1..=6).enumerate() {
        if index == 3 {
            new.push_str(&para("第3条の2 追加した条項。", Some("C0000001")));
        }
        new.push_str(&para(
            &format!("第{chapter}条 この条項の内容は {chapter} です。"),
            Some(&format!("A00000{chapter:02}")),
        ));
    }

    let diff = compare(&old, &new);
    assert_eq!(reported(&diff.summary), vec![("paragraphs_added", 1)]);
}

#[test]
fn deleting_a_paragraph_reports_one_removal() {
    let old = chapters(6);
    let new = old.replace(
        &para("第4条 この条項の内容は 4 です。", Some("A0000004")),
        "",
    );

    let diff = compare(&old, &new);
    assert_eq!(reported(&diff.summary), vec![("paragraphs_removed", 1)]);
}

#[test]
fn moving_a_paragraph_is_a_move_not_an_add_and_a_delete() {
    let old = chapters(5);
    let moved = para("第2条 この条項の内容は 2 です。", Some("A0000002"));
    let new = format!("{}{moved}", old.replace(&moved, ""));

    let diff = compare(&old, &new);
    assert_eq!(reported(&diff.summary), vec![("paragraphs_moved", 1)]);
}

#[test]
fn documents_without_para_ids_still_match_on_their_text() {
    // Word 以外が書き出した docx には paraId が無い。本文の一致で錨を打てること。
    let old: String = (1..=5)
        .map(|n| para(&format!("第{n}条 内容 {n}"), None))
        .collect();
    let new = old.replace("第3条 内容 3", "第3条 内容 3 (改)");

    let diff = compare(&old, &new);
    assert_eq!(reported(&diff.summary), vec![("paragraphs_changed", 1)]);
    assert!(
        diff.diagnostics.para_ids_missing(),
        "paraId が無いことを結果に添えること"
    );
}

#[test]
fn changing_one_table_cell_reports_one_row() {
    let rows: &[&[&str]] = &[
        &["条", "内容"],
        &["第1条", "目的"],
        &["第2条", "定義"],
        &["第3条", "期間"],
    ];
    let old = table(rows);
    let new = table(&[
        &["条", "内容"],
        &["第1条", "目的"],
        &["第2条", "用語の定義"],
        &["第3条", "期間"],
    ]);

    let diff = compare(&old, &new);
    assert_eq!(
        reported(&diff.summary),
        vec![("paragraphs_changed", 1), ("table_rows_changed", 1)]
    );
}

#[test]
fn a_row_is_matched_on_the_whole_row_not_just_its_first_cell() {
    // 1 列目が同じで 2 列目だけ違う行が並ぶ表。1 列目だけで行を見分けると
    // 取り違える形で、対照表そのものがこの形をしている。
    let old = table(&[&["改正", "第1回"], &["改正", "第2回"], &["改正", "第3回"]]);
    let new = table(&[&["改正", "第1回"], &["改正", "第2回"], &["改正", "第4回"]]);

    let diff = compare(&old, &new);
    assert_eq!(
        reported(&diff.summary),
        vec![("paragraphs_changed", 1), ("table_rows_changed", 1)],
        "変わったのは 3 行目だけのはず"
    );

    let jade_docx_diff::BlockChange::Table(change) = &diff.blocks[0] else {
        panic!("表のはず");
    };
    let changed: Vec<_> = change
        .rows
        .iter()
        .filter(|row| row.kind == ChangeKind::Changed)
        .filter_map(|row| row.new_index)
        .collect();
    assert_eq!(changed, vec![3]);
}

#[test]
fn inserting_a_table_row_reports_one_addition() {
    let old = table(&[&["第1条", "目的"], &["第2条", "定義"]]);
    let new = table(&[
        &["第1条", "目的"],
        &["第1条の2", "適用範囲"],
        &["第2条", "定義"],
    ]);

    let diff = compare(&old, &new);
    assert_eq!(reported(&diff.summary), vec![("table_rows_added", 1)]);
}

#[test]
fn a_whole_new_table_is_not_counted_row_by_row() {
    let old = para("本文", Some("D0000001"));
    let new = format!(
        "{}{}",
        para("本文", Some("D0000001")),
        table(&[&["A", "B"], &["C", "D"], &["E", "F"]])
    );

    let diff = compare(&old, &new);
    assert_eq!(reported(&diff.summary), vec![("tables_added", 1)]);
}

#[test]
fn a_nested_table_is_compared_inside_its_cell() {
    let nested = format!(
        "<w:tbl><w:tr><w:tc>{}<w:tbl><w:tr><w:tc>{}</w:tc></w:tr></w:tbl></w:tc></w:tr></w:tbl>",
        para("外側", None),
        para("内側の値 100", None)
    );
    let changed = nested.replace("内側の値 100", "内側の値 200");

    let diff = compare(&nested, &changed);
    assert_eq!(
        reported(&diff.summary),
        vec![("paragraphs_changed", 1), ("table_rows_changed", 1)],
        "外側の行まで数えると 1 箇所の直しが 2 行に見える"
    );
}

#[test]
fn tracked_changes_are_compared_after_accepting_them() {
    // 旧は履歴なしの完成形、新は同じ結果に至る履歴つき。受け入れ後で比べる
    // ので、差分は出ないのが正しい。
    let old = para("委託料は月額 600,000 円とする。", Some("E0000001"));
    let new = concat!(
        r#"<w:p w14:paraId="E0000001">"#,
        r#"<w:r><w:t xml:space="preserve">委託料は月額 </w:t></w:r>"#,
        r#"<w:del><w:r><w:delText>500,000</w:delText></w:r></w:del>"#,
        r#"<w:ins><w:r><w:t>600,000</w:t></w:r></w:ins>"#,
        r#"<w:r><w:t xml:space="preserve"> 円とする。</w:t></w:r>"#,
        "</w:p>"
    );

    let diff = compare(&old, new);
    assert!(
        !diff.has_changes(),
        "履歴を受け入れた後は同じ: {:?}",
        reported(&diff.summary)
    );
    assert!(
        diff.diagnostics.has_tracked_changes(),
        "履歴が残っていることを結果に添えること"
    );
}

#[test]
fn the_character_breakdown_points_at_what_changed() {
    let old = para("委託料は月額 500,000 円とする。", Some("F0000001"));
    let new = para("委託料は月額 550,000 円とする。", Some("F0000001"));

    let diff = compare(&old, &new);
    let jade_docx_diff::BlockChange::Paragraph(change) = &diff.blocks[0] else {
        panic!("段落のはず");
    };

    let inserted: String = change
        .segments
        .iter()
        .filter(|segment| segment.kind == jade_docx_diff::SegmentKind::Insert)
        .map(|segment| segment.text.as_str())
        .collect();
    assert_eq!(inserted, "5");
}

#[test]
fn a_file_that_is_not_a_docx_is_reported_readably() {
    let error =
        load_document_bytes(b"not a zip at all", &Limits::default()).expect_err("読めないはず");
    let message = error.to_string();
    assert!(message.len() < 220, "長すぎる: {message}");
}

#[test]
fn the_body_text_leaves_out_the_list_number() {
    // 番号は numbering.xml から組み立てるもので、文書の中に run としては
    // 存在しない。描画された段落へ装飾を重ねる側が突き合わせられるのは、
    // 番号を含まないほうだけ。
    let numbering = r#"<w:numbering
      xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
      <w:abstractNum w:abstractNumId="0">
        <w:lvl w:ilvl="0">
          <w:start w:val="1"/>
          <w:numFmt w:val="decimal"/>
          <w:lvlText w:val="%1."/>
        </w:lvl>
      </w:abstractNum>
      <w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
    </w:numbering>"#;
    let document = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr>
      <w:r><w:t>最初の項目</w:t></w:r>
    </w:p>
  </w:body>
</w:document>"#;

    let paragraphs = jade_docx_extract::extract_from_xml_parts(document, Some(numbering), None)
        .expect("extract");
    assert_eq!(paragraphs[0].accepted_numbering_text, "1.");

    let ir =
        jade_docx_diff::load_document_bytes(&super_docx(document, numbering), &Limits::default())
            .expect("読み込み");
    let jade_docx_diff::BlockIr::Paragraph(para) = &ir.blocks[0] else {
        panic!("段落のはず");
    };
    assert_eq!(para.text, "1.最初の項目", "番号つきのほう");
    assert_eq!(para.body_text, "最初の項目", "番号なしのほう");
}

/// numbering.xml も入れた docx を組み立てる。
fn super_docx(document_xml: &str, numbering_xml: &str) -> Vec<u8> {
    let mut buffer = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut buffer));
        let options: zip::write::SimpleFileOptions =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("[Content_Types].xml", options).unwrap();
        zip.write_all(br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#).unwrap();
        zip.start_file("_rels/.rels", options).unwrap();
        zip.write_all(br#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#).unwrap();
        zip.start_file("word/document.xml", options).unwrap();
        zip.write_all(document_xml.as_bytes()).unwrap();
        zip.start_file("word/numbering.xml", options).unwrap();
        zip.write_all(numbering_xml.as_bytes()).unwrap();
        zip.finish().unwrap();
    }
    buffer
}
