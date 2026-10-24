//! 旧と新の docx を、変更履歴付きの 1 つの docx に合成する。
//!
//! 「変更後のみ」では消えた条項が見えない。かといって画面に赤い枠で挟み込むと、
//! 実際の紙面とは違う見え方になる。Word が昔から持っている変更履歴
//! (`w:ins` / `w:del`) に落とし込めば、**本物の組版のまま**入った文字と消えた
//! 文字を並べられる。描く側も既存の変更履歴表示に乗せるだけで済む。
//!
//! 中身は新しい側の docx を土台にして `word/document.xml` だけ差し替える。
//! スタイルや画像は新しい側のものをそのまま使う。
//!
//! # 限り
//!
//! 消えた段落は旧の側から XML ごと持ってくる。旧にしかないスタイルや画像を
//! 参照していると、そこは落ちる (新の側の定義で描かれる)。
//!
//! 書き換わった段落は文字単位で組み直すので、段落の途中で書体が変わって
//! いる場合はそこまで保てない (`paragraph` の頭に書いてある)。

mod fragment;
mod paragraph;
mod revision;

use std::collections::HashMap;
use std::io::{Cursor, Read, Seek, Write};

use anyhow::{anyhow, Context, Result};
use docx_diff::{BlockChange, ChangeKind, DocumentDiff, RowChange};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use fragment::{split_body, Body, Fragment};
use revision::{mark_row, mark_runs, Mode, RevisionIds};

const DOCUMENT_PART: &str = "word/document.xml";

/// 旧と新から、変更履歴付きの docx を組み立てる。
pub fn merge(old_docx: &[u8], new_docx: &[u8], diff: &DocumentDiff) -> Result<Vec<u8>> {
    merge_with(old_docx, new_docx, diff, RevisionIds::default())
}

/// [`merge`] with the changes put down to `author` at `date` (an ISO 8601
/// time), as Word's 文書の比較 labels them with the name it is given.
pub fn merge_labelled(
    old_docx: &[u8],
    new_docx: &[u8],
    diff: &DocumentDiff,
    author: &str,
    date: &str,
) -> Result<Vec<u8>> {
    merge_with(
        old_docx,
        new_docx,
        diff,
        RevisionIds::labelled(author, date),
    )
}

fn merge_with(
    old_docx: &[u8],
    new_docx: &[u8],
    diff: &DocumentDiff,
    ids: RevisionIds,
) -> Result<Vec<u8>> {
    let old_xml = read_part(old_docx, DOCUMENT_PART).context("旧の document.xml")?;
    let new_xml = read_part(new_docx, DOCUMENT_PART).context("新の document.xml")?;
    let old_body = split_body(&old_xml).context("旧の本文")?;
    let new_body = split_body(&new_xml).context("新の本文")?;

    let merged = merge_body(&old_body, &new_body, diff, ids)?;
    repack(new_docx, &merged)
}

/// 本文だけを合成する。zip を触らないので試験しやすい。
///
/// 並べる順は**新の文書の並び**を軸にする。差分のブロックの並びは文書順とは
/// 限らない (表がまとめて先に来る)。消えたものは、旧の文書で直前にあった
/// 「新にも残っているもの」の後ろへ差し込む。
fn merge_body(old: &Body, new: &Body, diff: &DocumentDiff, ids: RevisionIds) -> Result<String> {
    // 番号から差分を引けるようにする。
    let mut para_by_new: HashMap<usize, &BlockChange> = HashMap::new();
    let mut table_by_new: HashMap<usize, &BlockChange> = HashMap::new();
    let mut para_by_old: HashMap<usize, &BlockChange> = HashMap::new();
    let mut table_by_old: HashMap<usize, &BlockChange> = HashMap::new();
    for block in &diff.blocks {
        match block {
            BlockChange::Paragraph(change) => {
                if let Some(i) = change.new_index {
                    para_by_new.insert(i, block);
                }
                if let Some(i) = change.old_index {
                    para_by_old.insert(i, block);
                }
            }
            BlockChange::Table(change) => {
                if let Some(i) = change.new_index {
                    table_by_new.insert(i, block);
                }
                if let Some(i) = change.old_index {
                    table_by_old.insert(i, block);
                }
            }
        }
    }

    // 旧の並びを辿り、消えたものを「直前の残っているもの」に紐づける。
    let mut leading: Vec<&Fragment> = Vec::new();
    let mut trailing: HashMap<usize, Vec<&Fragment>> = HashMap::new();
    {
        let mut anchor: Option<usize> = None;
        let mut paragraphs = 0usize;
        let mut tables = 0usize;
        for child in &old.children {
            let count = child.paragraph_count();
            let block = if child.is_paragraph() {
                paragraphs += 1;
                para_by_old.get(&paragraphs).copied()
            } else if child.is_table() {
                tables += 1;
                paragraphs += count;
                table_by_old.get(&tables).copied()
            } else {
                paragraphs += count;
                None
            };
            if !child.is_table() && !child.is_paragraph() {
                continue;
            }
            match block.map(|b| b.kind()) {
                Some(ChangeKind::Removed) | None => match anchor {
                    Some(at) => trailing.entry(at).or_default().push(child),
                    None => leading.push(child),
                },
                Some(_) => {
                    // 新にも残っているもの。ここから後ろの消えたものは、
                    // この要素の後ろへ置く。
                    let new_position = match (child.is_paragraph(), block) {
                        (true, Some(BlockChange::Paragraph(c))) => c.new_index.map(Key::Paragraph),
                        (false, Some(BlockChange::Table(c))) => c.new_index.map(Key::Table),
                        _ => None,
                    };
                    if let Some(key) = new_position {
                        anchor = Some(key.hash());
                    }
                }
            }
        }
    }

    let mut ids = ids;
    let mut out = String::with_capacity(new.prologue.len() + new.epilogue.len() + 4096);
    out.push_str(&new.prologue);
    for child in &leading {
        out.push_str(&mark_runs(&child.xml, Mode::Delete, &mut ids)?);
    }

    let mut paragraphs = 0usize;
    let mut tables = 0usize;
    for child in &new.children {
        let count = child.paragraph_count();
        let (block, key) = if child.is_paragraph() {
            paragraphs += 1;
            (
                para_by_new.get(&paragraphs).copied(),
                Some(Key::Paragraph(paragraphs)),
            )
        } else if child.is_table() {
            tables += 1;
            paragraphs += count;
            (table_by_new.get(&tables).copied(), Some(Key::Table(tables)))
        } else {
            paragraphs += count;
            (None, None)
        };

        match block {
            Some(BlockChange::Paragraph(change)) => match change.kind {
                ChangeKind::Added => out.push_str(&mark_runs(&child.xml, Mode::Insert, &mut ids)?),
                ChangeKind::Changed => out.push_str(&paragraph::rebuild_changed(
                    &child.xml,
                    &change.segments,
                    &mut ids,
                )?),
                // 移動と無変更はそのまま。移動そのものは一覧と左右の枠で見える。
                _ => out.push_str(&child.xml),
            },
            Some(BlockChange::Table(change)) => {
                let old_table = change.old_index.and_then(|i| old.table(i));
                out.push_str(&merge_table(
                    old_table.map(|f| f.xml.as_str()),
                    Some(&child.xml),
                    &change.rows,
                    &mut ids,
                )?);
            }
            // 差分に出てこないもの (sectPr の前の空段落など) はそのまま。
            None => out.push_str(&child.xml),
        }

        if let Some(key) = key {
            if let Some(removed) = trailing.get(&key.hash()) {
                for child in removed {
                    out.push_str(&mark_runs(&child.xml, Mode::Delete, &mut ids)?);
                }
            }
        }
    }

    out.push_str(&new.epilogue);
    Ok(out)
}

/// 新の側での置き場所。段落と表で番号がぶつからないようにする。
#[derive(Clone, Copy)]
enum Key {
    Paragraph(usize),
    Table(usize),
}

impl Key {
    fn hash(self) -> usize {
        match self {
            Key::Paragraph(i) => i * 2,
            Key::Table(i) => i * 2 + 1,
        }
    }
}

/// 表の中を行単位で合成する。
fn merge_table(
    old_table: Option<&str>,
    new_table: Option<&str>,
    rows: &[RowChange],
    ids: &mut RevisionIds,
) -> Result<String> {
    let Some(new_table) = new_table else {
        // 新の側が無いなら、旧をまるごと削除として出す。
        return match old_table {
            Some(xml) => mark_runs(xml, Mode::Delete, ids),
            None => Ok(String::new()),
        };
    };
    let new_rows = rows_of(new_table);
    let old_rows = old_table.map(rows_of).unwrap_or_default();
    let head_end = new_table
        .find("<w:tr")
        .unwrap_or_else(|| new_table.len().saturating_sub("</w:tbl>".len()));

    let mut out = String::new();
    out.push_str(&new_table[..head_end]);
    let mut old_cursor = 0usize;
    let mut new_cursor = 0usize;

    for row in rows {
        let old_row = if row.old_index.is_some() {
            take_str(&old_rows, &mut old_cursor)
        } else {
            None
        };
        let new_row = if row.new_index.is_some() {
            take_str(&new_rows, &mut new_cursor)
        } else {
            None
        };
        match row.kind {
            ChangeKind::Removed => {
                if let Some(xml) = old_row {
                    out.push_str(&mark_row(&xml, Mode::Delete, ids)?);
                }
            }
            ChangeKind::Added => {
                if let Some(xml) = new_row {
                    out.push_str(&mark_row(&xml, Mode::Insert, ids)?);
                }
            }
            // 行そのものは残っている。中のセルに書き換えがあれば、そこを
            // 組み直す。ここを飛ばすと、表の中の変更が統合表示に出ない。
            _ => match (new_row, old_row) {
                (Some(xml), _) => out.push_str(&merge_row(&xml, row, ids)?),
                (None, Some(xml)) => out.push_str(&xml),
                (None, None) => {}
            },
        }
    }
    out.push_str("</w:tbl>");
    Ok(out)
}

/// 行の中のセルを、書き換えのあるものだけ組み直す。
///
/// 行が残っていても、セルの中の文字が変わっていることがある。そこを見ないと
/// 表の中の変更が統合表示に出ない。
fn merge_row(row_xml: &str, row: &RowChange, ids: &mut RevisionIds) -> Result<String> {
    let fragments = children_of(row_xml, "w:tc");
    if fragments.is_empty() {
        return Ok(row_xml.to_string());
    }

    let mut out = String::new();
    let mut at = 0usize;
    for (index, cell) in fragments.iter().enumerate() {
        // セルとセルの間 (trPr など) はそのまま。
        out.push_str(&row_xml[at..cell.start]);
        let xml = &row_xml[cell.start..cell.end];
        match row.cells.get(index) {
            Some(change) => out.push_str(&merge_cell(xml, &change.blocks, ids)?),
            None => out.push_str(xml),
        }
        at = cell.end;
    }
    out.push_str(&row_xml[at..]);
    Ok(out)
}

/// セルの中の段落を、書き換えのあるものだけ組み直す。
fn merge_cell(cell_xml: &str, blocks: &[BlockChange], ids: &mut RevisionIds) -> Result<String> {
    let fragments = children_of(cell_xml, "w:p");
    if fragments.is_empty() {
        return Ok(cell_xml.to_string());
    }
    // 段落の変更だけを、出てくる順に見る (セルの中の表までは追わない)。
    let changes: Vec<&BlockChange> = blocks
        .iter()
        .filter(|block| matches!(block, BlockChange::Paragraph(_)))
        .collect();

    let mut out = String::new();
    let mut at = 0usize;
    for (index, paragraph) in fragments.iter().enumerate() {
        out.push_str(&cell_xml[at..paragraph.start]);
        let xml = &cell_xml[paragraph.start..paragraph.end];
        match changes.get(index) {
            Some(BlockChange::Paragraph(change)) => match change.kind {
                ChangeKind::Changed => {
                    out.push_str(&paragraph::rebuild_changed(xml, &change.segments, ids)?)
                }
                ChangeKind::Added => out.push_str(&mark_runs(xml, Mode::Insert, ids)?),
                ChangeKind::Removed => out.push_str(&mark_runs(xml, Mode::Delete, ids)?),
                _ => out.push_str(xml),
            },
            _ => out.push_str(xml),
        }
        at = paragraph.end;
    }
    out.push_str(&cell_xml[at..]);
    Ok(out)
}

/// ある要素の直下にある `tag` の位置を順に返す。入れ子のものは拾わない。
struct Span {
    start: usize,
    end: usize,
}

fn children_of(xml: &str, tag: &str) -> Vec<Span> {
    let open = format!("<{tag}");
    let close = format!("</{tag}");
    let mut spans = Vec::new();
    let bytes = xml.as_bytes();
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut start = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let Some(gt) = xml[i..].find('>').map(|at| i + at) else {
            break;
        };
        let head = &xml[i..=gt];
        // `<w:p` は `<w:pPr` にも当たる。閉じ側も `</w:tc` が `</w:tcPr` に
        // 当たる。どちらも次の 1 文字で見分ける。
        let boundary = |at: usize| matches!(bytes.get(at), Some(b'>') | Some(b' ') | Some(b'/'));
        let is_open = head.starts_with(&open) && boundary(i + open.len());
        let is_close = head.starts_with(&close) && boundary(i + close.len());
        if is_open && head.ends_with("/>") {
            if depth == 0 {
                spans.push(Span {
                    start: i,
                    end: gt + 1,
                });
            }
        } else if is_open {
            if depth == 0 {
                start = i;
            }
            depth += 1;
        } else if is_close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                spans.push(Span { start, end: gt + 1 });
            }
        }
        i = gt + 1;
    }
    spans
}

/// 表の直下の `w:tr` を順に切り出す。入れ子の表の行は拾わない。
fn rows_of(table_xml: &str) -> Vec<String> {
    let mut rows = Vec::new();
    let bytes = table_xml.as_bytes();
    let mut i = 0usize;
    let mut depth_tbl = 0usize;
    let mut depth_tr = 0usize;
    let mut start = 0usize;

    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let Some(close) = table_xml[i..].find('>').map(|at| i + at) else {
            break;
        };
        let tag = &table_xml[i..=close];
        let self_closing = tag.ends_with("/>");
        if tag.starts_with("<w:tbl") && !self_closing {
            depth_tbl += 1;
        } else if tag.starts_with("</w:tbl") {
            depth_tbl = depth_tbl.saturating_sub(1);
        } else if tag.starts_with("<w:tr") && !self_closing {
            // 入れ子の表 (depth_tbl > 1) の行は、その表の XML の一部として
            // 親の行に含めて出す。
            if depth_tbl == 1 && depth_tr == 0 {
                start = i;
            }
            if depth_tbl == 1 {
                depth_tr += 1;
            }
        } else if tag.starts_with("</w:tr") && depth_tbl == 1 {
            depth_tr = depth_tr.saturating_sub(1);
            if depth_tr == 0 {
                rows.push(table_xml[start..=close].to_string());
            }
        }
        i = close + 1;
    }
    rows
}

fn take_str(items: &[String], cursor: &mut usize) -> Option<String> {
    let item = items.get(*cursor).cloned();
    if item.is_some() {
        *cursor += 1;
    }
    item
}

fn read_part(docx: &[u8], name: &str) -> Result<String> {
    let mut archive = ZipArchive::new(Cursor::new(docx))?;
    let mut file = archive
        .by_name(name)
        .with_context(|| format!("{name} が見つかりません"))?;
    let mut out = String::new();
    file.read_to_string(&mut out)?;
    Ok(out)
}

/// 新の docx をそのまま写しつつ、document.xml だけ差し替える。
fn repack(new_docx: &[u8], document_xml: &str) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(new_docx))?;
    let buffer = Cursor::new(Vec::new());
    let mut writer = ZipWriter::new(buffer);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name()?.into_owned();
        if entry.is_dir() {
            continue;
        }
        writer.start_file(&name, options)?;
        if name == DOCUMENT_PART {
            writer.write_all(document_xml.as_bytes())?;
        } else {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            writer.write_all(&bytes)?;
        }
    }

    let mut cursor = writer.finish()?;
    cursor.rewind()?;
    let mut out = Vec::new();
    cursor.read_to_end(&mut out)?;
    if out.is_empty() {
        return Err(anyhow!("合成した docx が空になりました"));
    }
    Ok(out)
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn 表の直下の行だけを切り出す() {
        let xml = concat!(
            "<w:tbl><w:tr><w:tc>",
            "<w:tbl><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>",
            "</w:tc></w:tr><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>",
        );
        let rows = rows_of(xml);
        assert_eq!(rows.len(), 2, "入れ子の行まで拾っている: {rows:?}");
        assert!(rows[0].contains("<w:tbl>"), "入れ子の表が抜けた");
    }
}
