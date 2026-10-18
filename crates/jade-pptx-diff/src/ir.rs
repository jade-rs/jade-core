//! pptx から読み取った中間表現。
//!
//! 差分計算と描画の両方がこの IR を使う。描画側 (アプリの UI) にそのまま
//! JSON で渡すので、フィールドは「見た目を再現するのに要るもの」と
//! 「差分を取るのに要るもの」の和になっている。
//!
//! 長さの単位は EMU (English Metric Unit, 1 inch = 914400 EMU)。OOXML が
//! そのまま持っている値なので、変換せずに保持して描画側で割る。

use serde::Serialize;

/// プレゼンテーション 1 ファイル分。
#[derive(Debug, Clone, Serialize)]
pub struct PresentationIr {
    /// スライドの表示サイズ (EMU)。
    pub width: i64,
    pub height: i64,
    pub slides: Vec<SlideIr>,
    /// 画像の実体。キーは [`ImageIr::media_id`]、値は data URL。
    pub media: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SlideIr {
    /// zip 内のパート名 (`ppt/slides/slide1.xml`)。診断用。
    pub part: String,
    /// 1 始まりの表示順。
    pub index: usize,
    pub shapes: Vec<ShapeIr>,
    /// ノート欄のテキスト (段落を改行で連結)。
    pub notes: Option<String>,
}

/// 図形の種類。差分の粒度を決めるのに使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    /// テキストを持つ図形 (プレースホルダ含む)。
    Text,
    Table,
    Picture,
    /// 折れ線などテキストを持たない図形。
    Graphic,
}

/// 図形の位置とサイズ (EMU)。プレースホルダはレイアウト/マスタから継承する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Frame {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShapeIr {
    /// スライド内で一意な図形 ID (`p:cNvPr@id`)。
    pub id: String,
    pub name: String,
    pub kind: ShapeKind,
    /// プレースホルダ種別 (`title` / `body` / `ctrTitle` など) と番号。
    pub placeholder: Option<Placeholder>,
    /// 解決後の位置。継承しても分からなければ None。
    pub frame: Option<Frame>,
    /// 描画順 (spTree 内の出現順、0 始まり)。
    pub z: usize,
    /// 塗り・線・形状・回転など、位置とは独立した見た目の正規化値。
    /// 差分計算専用で、描画用 JSON には含めない。
    #[serde(skip)]
    pub appearance_signature: String,
    pub paragraphs: Vec<ParagraphIr>,
    pub table: Option<TableIr>,
    pub image: Option<ImageIr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Placeholder {
    /// `type` 属性。無指定は OOXML 既定の `body`。
    pub kind: String,
    /// `idx` 属性。
    pub index: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParagraphIr {
    /// 段落の全テキスト (run を連結したもの)。
    pub text: String,
    /// 箇条書きの階層 (`a:pPr@lvl`)。
    pub level: u8,
    /// 段落全体が太字か (最初の run の書式を代表値として持つ)。
    pub bold: bool,
    /// フォントサイズ (pt)。未指定なら None (描画側で既定値を当てる)。
    pub size: Option<f32>,
    /// 揃え (`l` / `ctr` / `r` / `just`)。
    pub align: Option<String>,
    /// 同じ書式が続く文字範囲。run の分割だけでは差分にならないよう連結済み。
    pub format_spans: Vec<FormatSpanIr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormatSpanIr {
    /// この書式が適用される Unicode scalar value の数。
    pub characters: usize,
    /// rPr を属性順に依存しない形へ正規化した値。
    pub style: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableIr {
    /// 行ごとのセルテキスト。
    pub rows: Vec<Vec<String>>,
    /// 列幅 (EMU)。描画用。
    pub column_widths: Vec<i64>,
    /// 行高 (EMU)。描画用。
    pub row_heights: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImageIr {
    /// [`PresentationIr::media`] を引くキー。
    pub media_id: String,
    /// 中身が同じ画像かどうかの判定に使う軽いハッシュ。
    pub content_hash: u64,
}

impl SlideIr {
    /// スライドの全テキスト。対応付けのスコア計算に使う。
    pub fn text(&self) -> String {
        let mut out = String::new();
        for shape in &self.shapes {
            for paragraph in &shape.paragraphs {
                if !paragraph.text.is_empty() {
                    out.push_str(&paragraph.text);
                    out.push('\n');
                }
            }
            if let Some(table) = &shape.table {
                for row in &table.rows {
                    out.push_str(&row.join("\t"));
                    out.push('\n');
                }
            }
        }
        out
    }

    /// タイトルプレースホルダのテキスト。一覧表示と対応付けに使う。
    pub fn title(&self) -> Option<String> {
        self.shapes
            .iter()
            .find(|shape| {
                shape
                    .placeholder
                    .as_ref()
                    .is_some_and(|ph| ph.kind == "title" || ph.kind == "ctrTitle")
            })
            .map(|shape| {
                shape
                    .paragraphs
                    .iter()
                    .map(|p| p.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|text| !text.trim().is_empty())
    }
}

impl ShapeIr {
    /// 図形の全テキスト。図形同士の対応付けに使う。
    pub fn text(&self) -> String {
        let mut out: Vec<&str> = self.paragraphs.iter().map(|p| p.text.as_str()).collect();
        if let Some(table) = &self.table {
            for row in &table.rows {
                for cell in row {
                    out.push(cell.as_str());
                }
            }
        }
        out.join("\n")
    }
}

/// 内容比較用の軽いハッシュ (FNV-1a)。暗号用途ではない。
pub(crate) fn content_hash(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
