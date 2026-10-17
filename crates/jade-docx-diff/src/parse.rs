//! docx を読んで [`DocumentIr`] にする。
//!
//! [`jade_docx_extract`] が返すのは段落の平らな並びで、1 つ 1 つが「どの表のどの行の
//! どのセルにいたか」(`table_path`) を持っている。ここではその道筋を頼りに、
//! 表と入れ子を組み直す。

use std::path::Path;

use anyhow::{Result, anyhow};
use jade_docx_extract::{Paragraph, TableCoord};

use crate::ir::{BlockIr, CellIr, DocumentIr, ParagraphIr, RowIr, TableIr};

/// 読み込み時の上限。
#[derive(Debug, Clone)]
pub struct Limits {
    /// 読み込むファイルサイズの上限 (MB)。
    pub max_input_mb: u64,
    /// 段落数の上限。
    pub max_paragraphs: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input_mb: 256,
            max_paragraphs: 200_000,
        }
    }
}

/// ファイルから読む。
pub fn load_document(path: &Path, limits: &Limits) -> Result<DocumentIr> {
    let size = std::fs::metadata(path)
        .map_err(|e| anyhow!("failed to read {}: {e}", path.display()))?
        .len();
    let max_bytes = limits.max_input_mb.saturating_mul(1024 * 1024);
    if size > max_bytes {
        return Err(anyhow!(
            "docx is {size} bytes, exceeding max_input_mb {}",
            limits.max_input_mb
        ));
    }

    let paragraphs = jade_docx_extract::extract_from_path(path)
        .map_err(|e| anyhow!("{}", readable_error(&e.to_string())))?;
    Ok(build(paragraphs, limits))
}

/// バイト列から読む。
pub fn load_document_bytes(bytes: &[u8], limits: &Limits) -> Result<DocumentIr> {
    let max_bytes = limits.max_input_mb.saturating_mul(1024 * 1024);
    if bytes.len() as u64 > max_bytes {
        return Err(anyhow!(
            "docx is {} bytes, exceeding max_input_mb {}",
            bytes.len(),
            limits.max_input_mb
        ));
    }

    let paragraphs = jade_docx_extract::extract_from_bytes(bytes)
        .map_err(|e| anyhow!("{}", readable_error(&e.to_string())))?;
    Ok(build(paragraphs, limits))
}

/// 平らな段落の並びを、表の入れ子ごと組み直す。
///
/// 段落は文書順に並んでいるので、`table_path` の前方一致を見ていけば木に
/// 戻せる。道筋が今いる場所より浅くなったら閉じ、深くなったら開く。
fn build(paragraphs: Vec<Paragraph>, limits: &Limits) -> DocumentIr {
    let mut blocks: Vec<BlockIr> = Vec::new();
    // 今開いている表の道筋。`open` の i 番目は `path` の i 段目に対応する。
    let mut open: Vec<OpenTable> = Vec::new();

    for (position, paragraph) in paragraphs
        .into_iter()
        .take(limits.max_paragraphs)
        .enumerate()
    {
        let Paragraph {
            text: original_body_text,
            numbering_text,
            accepted_text,
            accepted_numbering_text,
            table_path: path,
            in_textbox,
            in_shape,
            para_id,
            ..
        } = paragraph;

        // 今の道筋と食い違うところまで閉じる。
        let shared = shared_prefix(&open, &path);
        while open.len() > shared {
            let table = open.pop().expect("open は空ではない");
            let finished = BlockIr::Table(table.finish());
            push_block(&mut blocks, &mut open, finished);
        }

        // 足りないぶんを開く。
        for coord in &path[shared..] {
            let index = count_tables(&blocks, &open) + 1;
            open.push(OpenTable::new(index, *coord));
        }

        // 同じ表の中で行やセルが変わっていたら、その区切りを入れる。
        for (level, coord) in path.iter().enumerate() {
            open[level].advance_to(*coord);
        }

        let body_text = accepted_text.clone();
        let block = BlockIr::Paragraph(ParagraphIr {
            index: position + 1,
            para_id,
            text: merge_owned(accepted_numbering_text, accepted_text),
            body_text,
            original_text: merge_owned(numbering_text, original_body_text),
            in_textbox,
            in_shape,
        });
        push_block(&mut blocks, &mut open, block);
    }

    while let Some(table) = open.pop() {
        let finished = BlockIr::Table(table.finish());
        push_block(&mut blocks, &mut open, finished);
    }

    DocumentIr { blocks }
}

/// 今開いている表と、これから入る道筋で一致している段数。
///
/// 同じ表を指していても、行やセルが変わっただけなら開き直さない。表そのものが
/// 変わったときだけ閉じる。
fn shared_prefix(open: &[OpenTable], path: &[TableCoord]) -> usize {
    let mut shared = 0;
    while shared < open.len() && shared < path.len() {
        if open[shared].coord.table != path[shared].table {
            break;
        }
        shared += 1;
    }
    shared
}

/// 今いちばん内側の入れ物へ足す。表を開いていなければ本文へ。
fn push_block(blocks: &mut Vec<BlockIr>, open: &mut [OpenTable], block: BlockIr) {
    match open.last_mut() {
        Some(table) => table.push(block),
        None => blocks.push(block),
    }
}

/// これまでに見た表の数。通し番号を振るのに使う。
fn count_tables(blocks: &[BlockIr], open: &[OpenTable]) -> usize {
    fn walk(blocks: &[BlockIr]) -> usize {
        blocks
            .iter()
            .map(|block| match block {
                BlockIr::Paragraph(_) => 0,
                BlockIr::Table(table) => {
                    1 + table
                        .rows
                        .iter()
                        .flat_map(|row| row.cells.iter())
                        .map(|cell| walk(&cell.blocks))
                        .sum::<usize>()
                }
            })
            .sum()
    }
    walk(blocks) + open.iter().map(OpenTable::table_count).sum::<usize>()
}

/// 組み立て中の表。
struct OpenTable {
    index: usize,
    coord: TableCoord,
    rows: Vec<RowIr>,
    /// 今の行のセル。行が変わるときに `rows` へ移す。
    cells: Vec<CellIr>,
    /// 今のセルの中身。セルが変わるときに `cells` へ移す。
    blocks: Vec<BlockIr>,
}

impl OpenTable {
    fn new(index: usize, coord: TableCoord) -> Self {
        Self {
            index,
            coord,
            rows: Vec::new(),
            cells: Vec::new(),
            blocks: Vec::new(),
        }
    }

    /// 行やセルが変わっていたら、そこまでを閉じる。
    fn advance_to(&mut self, coord: TableCoord) {
        if coord.row != self.coord.row {
            self.close_cell();
            self.close_row();
        } else if coord.cell != self.coord.cell {
            self.close_cell();
        }
        self.coord = coord;
    }

    fn push(&mut self, block: BlockIr) {
        self.blocks.push(block);
    }

    /// 今のセルを、座標が示す列に置く。
    ///
    /// 段落が 1 つも入っていないセルは jade-docx-extract から返ってこない。数えた
    /// 順に詰めると、空のセルのぶんだけ以降の列がずれる。座標の列番号まで
    /// 空のセルで埋めてから置く。
    fn close_cell(&mut self) {
        while self.cells.len() < self.coord.cell {
            self.cells.push(CellIr {
                index: self.cells.len() + 1,
                blocks: Vec::new(),
            });
        }
        let blocks = std::mem::take(&mut self.blocks);
        self.cells.push(CellIr {
            index: self.cells.len() + 1,
            blocks,
        });
    }

    /// 今の行を、座標が示す位置に置く。空の行も同じ理由で埋める。
    fn close_row(&mut self) {
        while self.rows.len() < self.coord.row {
            self.rows.push(RowIr {
                index: self.rows.len() + 1,
                cells: Vec::new(),
            });
        }
        let cells = std::mem::take(&mut self.cells);
        self.rows.push(RowIr {
            index: self.rows.len() + 1,
            cells,
        });
    }

    fn finish(mut self) -> TableIr {
        self.close_cell();
        self.close_row();
        TableIr {
            index: self.index,
            rows: self.rows,
        }
    }

    /// この表と、その中で組み立て中の表の数。
    fn table_count(&self) -> usize {
        1
    }
}

/// 箇条書きの番号と本文をつなぐ。
fn merge_owned(mut numbering: String, text: String) -> String {
    match (numbering.is_empty(), text.is_empty()) {
        (true, _) => text,
        (false, true) => numbering,
        (false, false) => {
            numbering.reserve(text.len());
            numbering.push_str(&text);
            numbering
        }
    }
}

/// 解析器のエラーを人が読める長さに整える。
fn readable_error(message: &str) -> String {
    const LIMIT: usize = 200;

    if message.contains("word/document.xml") {
        return "docx として読めませんでした (word/document.xml がありません)".to_string();
    }
    let head: String = message
        .lines()
        .next()
        .unwrap_or(message)
        .chars()
        .take(LIMIT)
        .collect();
    if head.chars().count() < message.chars().count() {
        format!("{head}…")
    } else {
        head
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraph(text: &str, path: &[(usize, usize, usize)]) -> Paragraph {
        Paragraph {
            text: text.to_string(),
            numbering_text: String::new(),
            accepted_text: text.to_string(),
            accepted_numbering_text: String::new(),
            in_table: !path.is_empty(),
            table_path: path
                .iter()
                .map(|(table, row, cell)| TableCoord {
                    table: *table,
                    row: *row,
                    cell: *cell,
                })
                .collect(),
            in_textbox: false,
            in_shape: false,
            para_id: None,
        }
    }

    /// 木を "本文 / t1[r1c1: …]" のような読みやすい形に潰す。
    fn shape(blocks: &[BlockIr]) -> String {
        blocks
            .iter()
            .map(|block| match block {
                BlockIr::Paragraph(p) => p.text.clone(),
                BlockIr::Table(t) => {
                    let rows: Vec<String> = t
                        .rows
                        .iter()
                        .map(|row| {
                            let cells: Vec<String> =
                                row.cells.iter().map(|cell| shape(&cell.blocks)).collect();
                            format!("r{}[{}]", row.index, cells.join("|"))
                        })
                        .collect();
                    format!("t{}{{{}}}", t.index, rows.join(","))
                }
            })
            .collect::<Vec<_>>()
            .join(" / ")
    }

    #[test]
    fn body_paragraphs_stay_flat() {
        let document = build(
            vec![paragraph("第1条", &[]), paragraph("第2条", &[])],
            &Limits::default(),
        );
        assert_eq!(shape(&document.blocks), "第1条 / 第2条");
    }

    #[test]
    fn a_table_becomes_rows_and_cells() {
        let document = build(
            vec![
                paragraph("見出し", &[]),
                paragraph("A", &[(0, 0, 0)]),
                paragraph("B", &[(0, 0, 1)]),
                paragraph("C", &[(0, 1, 0)]),
                paragraph("D", &[(0, 1, 1)]),
                paragraph("あとがき", &[]),
            ],
            &Limits::default(),
        );
        assert_eq!(
            shape(&document.blocks),
            "見出し / t1{r1[A|B],r2[C|D]} / あとがき"
        );
    }

    #[test]
    fn two_tables_in_a_row_do_not_merge() {
        let document = build(
            vec![paragraph("A", &[(0, 0, 0)]), paragraph("B", &[(1, 0, 0)])],
            &Limits::default(),
        );
        assert_eq!(shape(&document.blocks), "t1{r1[A]} / t2{r1[B]}");
    }

    #[test]
    fn a_nested_table_sits_inside_its_cell() {
        let document = build(
            vec![
                paragraph("外", &[(0, 0, 0)]),
                paragraph("内", &[(0, 0, 0), (0, 0, 0)]),
                paragraph("外の続き", &[(0, 0, 1)]),
            ],
            &Limits::default(),
        );
        assert_eq!(shape(&document.blocks), "t1{r1[外 / t2{r1[内]}|外の続き]}");
    }

    #[test]
    fn paragraphs_are_numbered_across_the_whole_document() {
        let document = build(
            vec![
                paragraph("本文", &[]),
                paragraph("セル", &[(0, 0, 0)]),
                paragraph("あと", &[]),
            ],
            &Limits::default(),
        );
        assert_eq!(document.paragraph_count(), 3);
    }

    #[test]
    fn numbering_is_glued_to_the_body() {
        let mut para = paragraph("本文", &[]);
        para.accepted_numbering_text = "1. ".to_string();
        let document = build(vec![para], &Limits::default());
        assert_eq!(shape(&document.blocks), "1. 本文");
    }

    #[test]
    fn unnumbered_text_reuses_the_extracted_buffer() {
        let text = String::from("本文");
        let buffer = text.as_ptr();

        let merged = merge_owned(String::new(), text);

        assert_eq!(merged, "本文");
        assert_eq!(merged.as_ptr(), buffer);
    }

    #[test]
    fn an_empty_cell_keeps_the_columns_lined_up() {
        // 中身の無いセルは段落として返ってこない。数えた順に詰めると、
        // 空のセルのぶんだけ以降の列が左へずれて、比較が 1 列分ずれる。
        let document = build(
            vec![
                paragraph("左上", &[(0, 0, 0)]),
                // (0,0,1) は空。
                paragraph("右下", &[(0, 1, 1)]),
            ],
            &Limits::default(),
        );
        assert_eq!(shape(&document.blocks), "t1{r1[左上],r2[|右下]}");

        let BlockIr::Table(table) = &document.blocks[0] else {
            panic!("表のはず");
        };
        assert_eq!(table.rows[1].cells.len(), 2, "右下は 2 列目にいる");
        assert!(table.rows[1].cells[0].blocks.is_empty());
    }

    #[test]
    fn an_empty_row_keeps_the_rows_lined_up() {
        let document = build(
            vec![
                paragraph("1 行目", &[(0, 0, 0)]),
                // 2 行目は空。
                paragraph("3 行目", &[(0, 2, 0)]),
            ],
            &Limits::default(),
        );
        let BlockIr::Table(table) = &document.blocks[0] else {
            panic!("表のはず");
        };
        assert_eq!(table.rows.len(), 3);
        assert!(table.rows[1].cells.is_empty(), "2 行目は空のまま残る");
    }

    #[test]
    fn readable_error_explains_a_non_docx() {
        let out = readable_error("XML parse error: word/document.xml が見つかりません");
        assert!(out.contains("docx として読めません"), "{out}");
    }
}
