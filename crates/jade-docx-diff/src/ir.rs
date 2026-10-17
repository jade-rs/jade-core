//! docx を比較しやすい形に整える。
//!
//! 解析は [`jade_docx_extract`] (jlsi/jade) に任せる。あちらは変更履歴の
//! accept / reject と番号付けの再計算を持っていて、実ファイルのフィクスチャで
//! 検証されている。返ってくるのは段落の平らな並びなので、`table_path`
//! (どの表のどの行のどのセルか) を頼りに表の入れ子を組み直す。

use serde::Serialize;

/// 文書 1 つぶん。
#[derive(Debug, Clone, Serialize)]
pub struct DocumentIr {
    pub blocks: Vec<BlockIr>,
}

impl DocumentIr {
    /// 本文全体。対応付けのスコアに使う。
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(BlockIr::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 段落の総数 (表の中も数える)。
    pub fn paragraph_count(&self) -> usize {
        self.blocks.iter().map(BlockIr::paragraph_count).sum()
    }
}

/// 本文の並びに現れるもの。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BlockIr {
    Paragraph(ParagraphIr),
    Table(TableIr),
}

impl BlockIr {
    pub fn text(&self) -> String {
        match self {
            BlockIr::Paragraph(paragraph) => paragraph.text.clone(),
            BlockIr::Table(table) => table.text(),
        }
    }

    pub fn paragraph_count(&self) -> usize {
        match self {
            BlockIr::Paragraph(_) => 1,
            BlockIr::Table(table) => table
                .rows
                .iter()
                .flat_map(|row| row.cells.iter())
                .flat_map(|cell| cell.blocks.iter())
                .map(BlockIr::paragraph_count)
                .sum(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ParagraphIr {
    /// 文書内の通し番号 (1 始まり)。表の中のものも含めて数える。
    pub index: usize,
    /// Word が振る段落の識別子。同じ文書から派生した 2 つのファイルなら、
    /// 直していない段落はここが一致する。
    pub para_id: Option<String>,
    /// 箇条書きの番号を頭に付けた本文。変更履歴は反映済み (accept 後)。
    pub text: String,
    /// 番号を付けない本文。
    ///
    /// 番号は numbering.xml から組み立てられるもので、文書の中に run として
    /// 入っているわけではない。画面上の段落と突き合わせたい側 (装飾を重ねる
    /// など) が見るのはこちら。
    pub body_text: String,
    /// 変更履歴を反映する前の本文。`text` と違うなら履歴が残っている。
    pub original_text: String,
    pub in_textbox: bool,
    pub in_shape: bool,
}

impl ParagraphIr {
    /// 変更履歴が残ったままの段落か。
    pub fn has_tracked_changes(&self) -> bool {
        self.text != self.original_text
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TableIr {
    /// 文書内の通し番号 (1 始まり)。
    pub index: usize,
    pub rows: Vec<RowIr>,
}

impl TableIr {
    pub fn text(&self) -> String {
        self.rows
            .iter()
            .map(RowIr::text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 表を見分けるための見出し。最初に中身のある行を使う。
    pub fn signature(&self) -> String {
        self.rows
            .iter()
            .map(RowIr::text)
            .find(|text| !text.trim().is_empty())
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RowIr {
    /// 表の中での行番号 (1 始まり)。
    pub index: usize,
    pub cells: Vec<CellIr>,
}

impl RowIr {
    pub fn text(&self) -> String {
        self.cells
            .iter()
            .map(CellIr::text)
            .collect::<Vec<_>>()
            .join("\t")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CellIr {
    /// 行の中での列番号 (1 始まり)。
    pub index: usize,
    /// セルの中身。表の入れ子もここに入る。
    pub blocks: Vec<BlockIr>,
}

impl CellIr {
    pub fn text(&self) -> String {
        self.blocks
            .iter()
            .map(BlockIr::text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// 空白のゆれを畳んで、全角と半角も寄せる。
///
/// 日本語の文書では、同じ内容でも U+3000 (全角空白) と半角空白、全角英数と
/// 半角英数が混ざる。対応付けの鍵にはこれを通したものを使う。
///
/// 数字はそのまま残す。jade の新旧対照表は数字を 0 に潰して「2021 年」と
/// 「2019 年」を同じ鍵にしているが、あれは類似度による救済が無いぶん攻めた
/// 正規化で、こちらは似ているかどうかを別に測れるので数字は残しておく。
pub fn normalize(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;

    let folded: String = text.nfkc().collect();
    let mut out = String::with_capacity(folded.len());
    let mut pending_space = false;
    for ch in folded.chars() {
        if ch.is_whitespace() || ch == '\u{3000}' {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_folds_widths_and_spacing() {
        assert_eq!(normalize("ＡＢＣ　１２３"), "ABC 123");
        assert_eq!(normalize("  第 1 条  "), "第 1 条");
        assert_eq!(normalize("行\n継ぎ"), "行 継ぎ");
    }

    #[test]
    fn normalize_keeps_the_digits() {
        // 「2021 年」と「2019 年」は別物として残す。似ているかどうかは
        // 類似度のほうで見る。
        assert_ne!(normalize("2021年2月26日"), normalize("2019年8月26日"));
    }

    #[test]
    fn normalize_leaves_an_empty_string_empty() {
        assert_eq!(normalize("　 \n "), "");
    }
}
