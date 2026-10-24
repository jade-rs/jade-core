//! 変更履歴の「すべて承諾」を、Word の意味を写した小さなモデルと突き合わせる。
//!
//! 背景: 段落をまたいだ削除（段落記号の削除）という基本的な文書で `transform_docx_annotations` が
//! 失敗していた（jade#403）。個別の例では網羅できないので、段落・表・行・セル・content control・
//! ハイパーリンク・フィールド・コメント範囲・ブックマークと、その中の挿入/削除/移動を組み合わせた
//! 文書を生成し、次を確かめる。
//!
//! - 本文の文字列（段落ごと、文書順）がモデルと一致する
//! - 表・行・セル・段落の構造が WordprocessingML として有効なまま（空の表や行、段落で終わらないセルを作らない）
//! - 変更履歴・コメントの印が、外すと決めたものは残らない
//! - コメント範囲・ブックマーク・複合フィールドの開始と終了が対応したまま
//! - 変換は冪等で、どちらも残す指定では元のバイト列を返す

use jade_docx_reader::{DocxXmlDocument, XmlElement, XmlNode};
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

// ---------------------------------------------------------------------------
// 生成する文書のモデル
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Inline {
    Text,
    Ins(Vec<Inline>),
    Del(Vec<Inline>),
    MoveFrom(Vec<Inline>),
    MoveTo(Vec<Inline>),
    Hyperlink(Vec<Inline>),
    /// commentRangeStart … commentRangeEnd と、その後ろの commentReference の run。
    Comment(Vec<Inline>),
    Bookmark(Vec<Inline>),
    /// 複合フィールド。`deleted` は begin から end までを丸ごと削除したもの。
    Field {
        deleted: bool,
        result: Vec<Inline>,
    },
    /// 書式の変更履歴（rPrChange）が付いた run。
    FormatChange,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mark {
    Normal,
    Inserted,
    Deleted,
    MovedFrom,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Props {
    None,
    Center,
    Numbered,
    /// 段落書式の変更履歴（pPrChange）。
    Changed,
}

#[derive(Clone, Debug)]
struct Para {
    props: Props,
    mark: Mark,
    content: Vec<Inline>,
}

#[derive(Clone, Debug)]
enum Block {
    P(Para),
    Table(Vec<Row>),
    Sdt(Vec<Block>),
    /// 本文レベルに置いた bookmarkStart … bookmarkEnd（段落をまたぐブックマーク）。
    Bookmark(Vec<Block>),
    /// 本文レベルに置いた commentRangeStart … commentRangeEnd と、参照だけを持つ段落。
    Comment(Vec<Block>),
}

#[derive(Clone, Debug)]
struct Row {
    state: Mark,
    cells: Vec<Cell>,
}

#[derive(Clone, Debug)]
struct Cell {
    deleted: bool,
    /// 最後は必ず段落（セルは段落で終わる）。
    blocks: Vec<Block>,
}

fn inline() -> impl Strategy<Value = Inline> {
    let leaf = prop_oneof![4 => Just(Inline::Text), 1 => Just(Inline::FormatChange)];
    leaf.prop_recursive(3, 24, 4, |inner| {
        let list = prop::collection::vec(inner, 0..4);
        prop_oneof![
            list.clone().prop_map(Inline::Ins),
            list.clone().prop_map(Inline::Del),
            list.clone().prop_map(Inline::MoveFrom),
            list.clone().prop_map(Inline::MoveTo),
            list.clone().prop_map(Inline::Hyperlink),
            list.clone().prop_map(Inline::Comment),
            list.clone().prop_map(Inline::Bookmark),
            (any::<bool>(), list).prop_map(|(deleted, result)| Inline::Field { deleted, result }),
        ]
    })
}

fn mark() -> impl Strategy<Value = Mark> {
    prop_oneof![5 => Just(Mark::Normal), 1 => Just(Mark::Inserted), 3 => Just(Mark::Deleted), 1 => Just(Mark::MovedFrom)]
}

fn props() -> impl Strategy<Value = Props> {
    prop_oneof![3 => Just(Props::None), 1 => Just(Props::Center), 1 => Just(Props::Numbered), 1 => Just(Props::Changed)]
}

fn para() -> impl Strategy<Value = Para> {
    (props(), mark(), prop::collection::vec(inline(), 0..5)).prop_map(|(props, mark, content)| {
        Para {
            props,
            mark,
            content,
        }
    })
}

/// 段落で終わるブロック列（セル・本文の中身）。
fn blocks_ending_with_para(block: BoxedStrategy<Block>) -> impl Strategy<Value = Vec<Block>> {
    (prop::collection::vec(block, 0..4), para()).prop_map(|(mut blocks, last)| {
        blocks.push(Block::P(last));
        blocks
    })
}

fn block() -> BoxedStrategy<Block> {
    let leaf = para().prop_map(Block::P);
    leaf.prop_recursive(2, 32, 4, |inner| {
        let cell = (prop::bool::weighted(0.15), blocks_ending_with_para(inner.clone()))
            .prop_map(|(deleted, blocks)| Cell { deleted, blocks });
        let row = (
            prop_oneof![4 => Just(Mark::Normal), 1 => Just(Mark::Inserted), 2 => Just(Mark::Deleted)],
            prop::collection::vec(cell, 1..3),
        )
            .prop_map(|(state, cells)| Row { state, cells });
        prop_oneof![
            3 => para().prop_map(Block::P),
            2 => prop::collection::vec(row, 1..4).prop_map(Block::Table),
            1 => prop::collection::vec(inner.clone(), 1..3).prop_map(Block::Sdt),
            1 => prop::collection::vec(inner.clone(), 0..3).prop_map(Block::Bookmark),
            1 => prop::collection::vec(inner, 0..3).prop_map(Block::Comment),
        ]
    })
    .boxed()
}

fn body() -> impl Strategy<Value = Vec<Block>> {
    blocks_ending_with_para(block())
}

// ---------------------------------------------------------------------------
// XML に書き出す（本文の文字は通し番号で一意にして、順序も確かめられるようにする）
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Writer {
    xml: String,
    next: usize,
    /// 書いたコメントの w:id（comments.xml にコメントの中身を入れる）。
    comments: Vec<usize>,
}

impl Writer {
    fn id(&mut self) -> usize {
        self.next += 1;
        self.next
    }

    fn text(&mut self, deleted: bool) {
        let id = self.id();
        if deleted {
            self.xml
                .push_str(&format!("<w:r><w:delText>t{id}.</w:delText></w:r>"));
        } else {
            self.xml.push_str(&format!("<w:r><w:t>t{id}.</w:t></w:r>"));
        }
    }

    fn inlines(&mut self, items: &[Inline], deleted: bool) {
        for item in items {
            self.inline(item, deleted);
        }
    }

    fn inline(&mut self, item: &Inline, deleted: bool) {
        match item {
            Inline::Text => self.text(deleted),
            Inline::FormatChange => {
                let id = self.id();
                let tag = if deleted { "delText" } else { "t" };
                self.xml.push_str(&format!(
                    r#"<w:r><w:rPr><w:b/><w:rPrChange w:id="{id}" w:author="a"><w:rPr/></w:rPrChange></w:rPr><w:{tag}>t{id}.</w:{tag}></w:r>"#
                ));
            }
            Inline::Ins(items) | Inline::MoveTo(items) => {
                let name = if matches!(item, Inline::Ins(_)) {
                    "ins"
                } else {
                    "moveTo"
                };
                let id = self.id();
                self.xml
                    .push_str(&format!(r#"<w:{name} w:id="{id}" w:author="a">"#));
                self.inlines(items, deleted);
                self.xml.push_str(&format!("</w:{name}>"));
            }
            Inline::Del(items) | Inline::MoveFrom(items) => {
                let name = if matches!(item, Inline::Del(_)) {
                    "del"
                } else {
                    "moveFrom"
                };
                let id = self.id();
                self.xml
                    .push_str(&format!(r#"<w:{name} w:id="{id}" w:author="a">"#));
                self.inlines(items, true);
                self.xml.push_str(&format!("</w:{name}>"));
            }
            Inline::Hyperlink(items) => {
                self.xml.push_str(r#"<w:hyperlink w:anchor="x">"#);
                self.inlines(items, deleted);
                self.xml.push_str("</w:hyperlink>");
            }
            Inline::Comment(items) => {
                let id = self.id();
                self.comments.push(id);
                self.xml
                    .push_str(&format!(r#"<w:commentRangeStart w:id="{id}"/>"#));
                self.inlines(items, deleted);
                self.xml.push_str(&format!(
                    r#"<w:commentRangeEnd w:id="{id}"/><w:r><w:commentReference w:id="{id}"/></w:r>"#
                ));
            }
            Inline::Bookmark(items) => {
                let id = self.id();
                self.xml
                    .push_str(&format!(r#"<w:bookmarkStart w:id="{id}" w:name="b{id}"/>"#));
                self.inlines(items, deleted);
                self.xml
                    .push_str(&format!(r#"<w:bookmarkEnd w:id="{id}"/>"#));
            }
            Inline::Field {
                deleted: field_deleted,
                result,
            } => {
                let removed = deleted || *field_deleted;
                let (instr, open, close) = if *field_deleted && !deleted {
                    let id = self.id();
                    (
                        "delInstrText",
                        format!(r#"<w:del w:id="{id}" w:author="a">"#),
                        "</w:del>",
                    )
                } else if removed {
                    ("delInstrText", String::new(), "")
                } else {
                    ("instrText", String::new(), "")
                };
                self.xml.push_str(&open);
                self.xml.push_str(&format!(
                    r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:{instr}> REF x </w:{instr}></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>"#
                ));
                self.inlines(result, removed);
                self.xml
                    .push_str(r#"<w:r><w:fldChar w:fldCharType="end"/></w:r>"#);
                self.xml.push_str(close);
            }
        }
    }

    fn para(&mut self, para: &Para) {
        let mut properties = String::new();
        match para.props {
            Props::None => {}
            Props::Center => properties.push_str(r#"<w:jc w:val="center"/>"#),
            Props::Numbered => {
                properties.push_str(r#"<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>"#)
            }
            Props::Changed => {
                let id = self.id();
                properties.push_str(&format!(
                    r#"<w:jc w:val="right"/><w:pPrChange w:id="{id}" w:author="a"><w:pPr/></w:pPrChange>"#
                ));
            }
        }
        match para.mark {
            Mark::Normal => {}
            Mark::Inserted | Mark::Deleted | Mark::MovedFrom => {
                let name = match para.mark {
                    Mark::Inserted => "ins",
                    Mark::Deleted => "del",
                    _ => "moveFrom",
                };
                let id = self.id();
                properties.push_str(&format!(
                    r#"<w:rPr><w:{name} w:id="{id}" w:author="a"/></w:rPr>"#
                ));
            }
        }
        self.xml.push_str("<w:p>");
        if !properties.is_empty() {
            self.xml.push_str(&format!("<w:pPr>{properties}</w:pPr>"));
        }
        // 移動元の段落は、段落記号だけでなく中身も移動元として印が付く。
        if para.mark == Mark::MovedFrom {
            let id = self.id();
            self.xml
                .push_str(&format!(r#"<w:moveFrom w:id="{id}" w:author="a">"#));
            self.inlines(&para.content, true);
            self.xml.push_str("</w:moveFrom>");
        } else {
            self.inlines(&para.content, false);
        }
        self.xml.push_str("</w:p>");
    }

    fn blocks(&mut self, blocks: &[Block]) {
        for block in blocks {
            match block {
                Block::P(para) => self.para(para),
                Block::Bookmark(inner) => {
                    let id = self.id();
                    self.xml
                        .push_str(&format!(r#"<w:bookmarkStart w:id="{id}" w:name="b{id}"/>"#));
                    self.blocks(inner);
                    self.xml
                        .push_str(&format!(r#"<w:bookmarkEnd w:id="{id}"/>"#));
                }
                Block::Comment(inner) => {
                    let id = self.id();
                    self.comments.push(id);
                    self.xml
                        .push_str(&format!(r#"<w:commentRangeStart w:id="{id}"/>"#));
                    self.blocks(inner);
                    self.xml.push_str(&format!(
                        r#"<w:commentRangeEnd w:id="{id}"/><w:p><w:r><w:commentReference w:id="{id}"/></w:r></w:p>"#
                    ));
                }
                Block::Sdt(inner) => {
                    self.xml.push_str("<w:sdt><w:sdtPr/><w:sdtContent>");
                    self.blocks(inner);
                    self.xml.push_str("</w:sdtContent></w:sdt>");
                }
                Block::Table(rows) => {
                    self.xml.push_str(
                        "<w:tbl><w:tblPr/><w:tblGrid><w:gridCol/><w:gridCol/></w:tblGrid>",
                    );
                    for row in rows {
                        self.xml.push_str("<w:tr>");
                        match row.state {
                            Mark::Deleted | Mark::Inserted => {
                                let name = if row.state == Mark::Deleted {
                                    "del"
                                } else {
                                    "ins"
                                };
                                let id = self.id();
                                self.xml.push_str(&format!(
                                    r#"<w:trPr><w:{name} w:id="{id}" w:author="a"/></w:trPr>"#
                                ));
                            }
                            _ => {}
                        }
                        for cell in &row.cells {
                            self.xml.push_str("<w:tc>");
                            if cell.deleted {
                                let id = self.id();
                                self.xml.push_str(&format!(
                                    r#"<w:tcPr><w:cellDel w:id="{id}" w:author="a"/></w:tcPr>"#
                                ));
                            }
                            self.blocks(&cell.blocks);
                            self.xml.push_str("</w:tc>");
                        }
                        self.xml.push_str("</w:tr>");
                    }
                    self.xml.push_str("</w:tbl>");
                }
            }
        }
    }
}

/// WordprocessingML の書き方の揺れ。変換は接頭辞ではなく名前空間で要素を見分ける必要がある。
#[derive(Clone, Copy, Debug)]
enum Spelling {
    /// Word が書く `w:`。
    Word,
    /// 他のツールが書く任意の接頭辞。
    OtherPrefix,
    /// 要素は既定の名前空間、属性だけ `w:`。
    DefaultNamespace,
    /// ISO/IEC 29500 Strict の名前空間。
    Strict,
}

fn spelling() -> impl Strategy<Value = Spelling> {
    prop_oneof![
        4 => Just(Spelling::Word),
        1 => Just(Spelling::OtherPrefix),
        1 => Just(Spelling::DefaultNamespace),
        1 => Just(Spelling::Strict),
    ]
}

fn spell(xml: String, spelling: Spelling) -> String {
    match spelling {
        Spelling::Word => xml,
        Spelling::OtherPrefix => xml
            .replace("<w:", "<ns0:")
            .replace("</w:", "</ns0:")
            .replace(" w:", " ns0:")
            .replace("xmlns:w=", "xmlns:ns0="),
        Spelling::DefaultNamespace => xml.replace("<w:", "<").replace("</w:", "</").replace(
            &format!(r#"xmlns:w="{W}""#),
            &format!(r#"xmlns="{W}" xmlns:w="{W}""#),
        ),
        Spelling::Strict => xml.replace(W, "http://purl.oclc.org/ooxml/wordprocessingml/main"),
    }
}

/// 要素の間に改行と字下げを入れる（人手や他のツールが整形した XML）。空白だけのテキストノードができる。
fn pretty(xml: String) -> String {
    xml.replace("><", ">\n  <")
}

fn render(body: &[Block], spelling: Spelling, indented: bool) -> Vec<u8> {
    let mut writer = Writer::default();
    writer.blocks(body);
    let mut document = spell(
        format!(
            r#"<w:document xmlns:w="{W}"><w:body>{}<w:sectPr/></w:body></w:document>"#,
            writer.xml
        ),
        spelling,
    );
    if indented {
        document = pretty(document);
    }
    // コメントの中身も入れる。承諾で本文から参照ごと消えたコメントが comments.xml に残らないことを確かめる。
    let entries: String = writer
        .comments
        .iter()
        .map(|id| format!(r#"<w:comment w:id="{id}" w:author="a"><w:p><w:r><w:t>c{id}</w:t></w:r></w:p></w:comment>"#))
        .collect();
    let comments = spell(
        format!(r#"<w:comments xmlns:w="{W}">{entries}</w:comments>"#),
        spelling,
    );
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in [
        ("[Content_Types].xml", r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/></Types>"#.to_string()),
        ("word/_rels/document.xml.rels", r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="c" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/></Relationships>"#.to_string()),
        ("word/document.xml", document),
        ("word/comments.xml", comments),
    ] {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

// ---------------------------------------------------------------------------
// Word の「すべて承諾」のモデル
// ---------------------------------------------------------------------------

/// 承諾後の段落（文字列と、残る段落記号の書式）。表と content control は中身を展開する。
#[derive(Clone, Debug, PartialEq)]
struct Expected {
    text: String,
    props: Props,
}

#[derive(Default)]
struct Model {
    next: usize,
}

impl Model {
    fn id(&mut self) -> usize {
        self.next += 1;
        self.next
    }

    /// Writer と同じ順で番号を消費しながら、承諾後に残る文字列を返す。
    fn inlines(&mut self, items: &[Inline], deleted: bool, out: &mut String) {
        for item in items {
            match item {
                Inline::Text | Inline::FormatChange => {
                    let id = self.id();
                    if !deleted {
                        out.push_str(&format!("t{id}."));
                    }
                }
                Inline::Ins(items) | Inline::MoveTo(items) => {
                    self.id();
                    self.inlines(items, deleted, out);
                }
                Inline::Del(items) | Inline::MoveFrom(items) => {
                    self.id();
                    self.inlines(items, true, out);
                }
                Inline::Hyperlink(items) => self.inlines(items, deleted, out),
                Inline::Comment(items) | Inline::Bookmark(items) => {
                    self.id();
                    self.inlines(items, deleted, out);
                }
                Inline::Field {
                    deleted: field_deleted,
                    result,
                } => {
                    if *field_deleted && !deleted {
                        self.id();
                    }
                    self.inlines(result, deleted || *field_deleted, out);
                }
            }
        }
    }

    fn para(&mut self, para: &Para) -> (String, Props, bool) {
        if para.props == Props::Changed {
            self.id();
        }
        if para.mark != Mark::Normal {
            self.id();
        }
        let mut text = String::new();
        if para.mark == Mark::MovedFrom {
            self.id();
            self.inlines(&para.content, true, &mut text);
        } else {
            self.inlines(&para.content, false, &mut text);
        }
        let props = if para.props == Props::Changed {
            Props::Changed
        } else {
            para.props
        };
        (
            text,
            props,
            matches!(para.mark, Mark::Deleted | Mark::MovedFrom),
        )
    }

    /// ブロック列を承諾する。段落記号が消えた段落は後ろの段落へ連結し、
    /// 連結先が無ければ最後の段落を残してそこへ連結する。丸ごと消える表と範囲の印は連結を妨げない。
    fn blocks(&mut self, blocks: &[Block], out: &mut Vec<Expected>) {
        let mut pending = Pending::default();
        self.continue_blocks(blocks, &mut pending, out);
        pending.flush(out);
    }

    fn continue_blocks(
        &mut self,
        blocks: &[Block],
        pending: &mut Pending,
        out: &mut Vec<Expected>,
    ) {
        for block in blocks {
            match block {
                Block::P(para) => {
                    let (text, props, mark_removed) = self.para(para);
                    pending.text.push_str(&text);
                    if mark_removed {
                        pending.last = Some(props);
                    } else {
                        pending.last = None;
                        out.push(Expected {
                            text: std::mem::take(&mut pending.text),
                            props,
                        });
                    }
                }
                Block::Sdt(inner) => {
                    pending.flush(out);
                    self.blocks(inner, out);
                }
                Block::Bookmark(inner) => {
                    self.id();
                    self.continue_blocks(inner, pending, out);
                }
                Block::Comment(inner) => {
                    self.id();
                    self.continue_blocks(inner, pending, out);
                    // 参照だけを持つ段落。
                    pending.last = None;
                    out.push(Expected {
                        text: std::mem::take(&mut pending.text),
                        props: Props::None,
                    });
                }
                Block::Table(rows) => {
                    let mut inner = Vec::new();
                    let mut kept_rows = 0;
                    for row in rows {
                        if row.state != Mark::Normal {
                            self.id();
                        }
                        let mut kept_cells = 0;
                        let mut row_inner = Vec::new();
                        for cell in &row.cells {
                            if cell.deleted {
                                self.id();
                            }
                            let mut cell_inner = Vec::new();
                            self.blocks(&cell.blocks, &mut cell_inner);
                            if !cell.deleted {
                                kept_cells += 1;
                                row_inner.extend(cell_inner);
                            }
                        }
                        if row.state != Mark::Deleted && kept_cells > 0 {
                            kept_rows += 1;
                            inner.extend(row_inner);
                        }
                    }
                    if kept_rows > 0 {
                        pending.flush(out);
                        out.extend(inner);
                    }
                }
            }
        }
    }
}

/// 段落記号が消えて、後ろの段落への連結を待っている文字列。
#[derive(Default)]
struct Pending {
    text: String,
    /// 待っている最後の段落の書式。連結先が無ければこの段落が残る。
    last: Option<Props>,
}

impl Pending {
    fn flush(&mut self, out: &mut Vec<Expected>) {
        if let Some(props) = self.last.take() {
            out.push(Expected {
                text: std::mem::take(&mut self.text),
                props,
            });
        }
    }
}

fn expected_after_accepting(body: &[Block]) -> Vec<Expected> {
    let mut out = Vec::new();
    Model::default().blocks(body, &mut out);
    out
}

// ---------------------------------------------------------------------------
// 出力の検査
// ---------------------------------------------------------------------------

fn parts(data: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(data)).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut entry = archive.by_index(index).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            (entry.name().unwrap().into_owned(), bytes)
        })
        .collect()
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn elements(element: &XmlElement) -> impl Iterator<Item = &XmlElement> {
    element.children.iter().filter_map(|node| match node {
        XmlNode::Element(child) => Some(child),
        _ => None,
    })
}

fn visible_text(element: &XmlElement, out: &mut String) {
    for child in elements(element) {
        if local(&child.name) == "t" {
            for node in &child.children {
                if let XmlNode::Text(text) = node {
                    out.push_str(text);
                }
            }
        } else {
            visible_text(child, out);
        }
    }
}

fn props_of(paragraph: &XmlElement) -> Props {
    let Some(properties) = elements(paragraph).find(|e| local(&e.name) == "pPr") else {
        return Props::None;
    };
    let names: Vec<_> = elements(properties).map(|e| local(&e.name)).collect();
    let jc = elements(properties)
        .find(|e| local(&e.name) == "jc")
        .and_then(|e| e.attributes.iter().find(|a| local(&a.name) == "val"))
        .map(|a| a.value.as_str());
    if names.contains(&"numPr") {
        Props::Numbered
    } else if jc == Some("right") {
        Props::Changed
    } else if jc == Some("center") {
        Props::Center
    } else {
        Props::None
    }
}

/// 段落を文書順に集め、構造の有効性も確かめる。
fn collect(element: &XmlElement, out: &mut Vec<Expected>, problems: &mut Vec<String>) {
    let name = local(&element.name);
    match name {
        "p" => {
            let children: Vec<_> = elements(element).collect();
            if children.iter().skip(1).any(|e| local(&e.name) == "pPr") {
                problems.push("pPr is not the first child of a paragraph".into());
            }
            if children
                .iter()
                .any(|e| matches!(local(&e.name), "p" | "tbl"))
            {
                problems.push("paragraph contains a block".into());
            }
            let mut text = String::new();
            visible_text(element, &mut text);
            out.push(Expected {
                text,
                props: props_of(element),
            });
            return;
        }
        "tbl" => {
            if !elements(element).any(|e| local(&e.name) == "tr") {
                problems.push("table without rows".into());
            }
        }
        "tr" => {
            if !elements(element).any(|e| local(&e.name) == "tc") {
                problems.push("row without cells".into());
            }
        }
        "tc" => {
            if elements(element).last().map(|e| local(&e.name)) != Some("p") {
                problems.push("cell does not end with a paragraph".into());
            }
        }
        _ => {}
    }
    for child in elements(element) {
        collect(child, out, problems);
    }
}

fn names_in(element: &XmlElement, out: &mut Vec<(String, String)>) {
    let id = element
        .attributes
        .iter()
        .find(|a| local(&a.name) == "id" || local(&a.name) == "fldCharType")
        .map(|a| a.value.clone())
        .unwrap_or_default();
    out.push((local(&element.name).to_string(), id));
    for child in elements(element) {
        names_in(child, out);
    }
}

const REVISION_NAMES: &[&str] = &[
    "ins",
    "del",
    "moveFrom",
    "moveTo",
    "rPrChange",
    "pPrChange",
    "cellDel",
    "delText",
    "delInstrText",
];
const COMMENT_NAMES: &[&str] = &["commentRangeStart", "commentRangeEnd", "commentReference"];

fn check(
    source_body: &[Block],
    output: &[u8],
    include_comments: bool,
    include_revisions: bool,
) -> Result<(), TestCaseError> {
    let files = parts(output);
    let xml = std::str::from_utf8(&files["word/document.xml"]).unwrap();
    let doc = DocxXmlDocument::parse(xml).map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
    let mut paragraphs = Vec::new();
    let mut problems = Vec::new();
    collect(doc.root(), &mut paragraphs, &mut problems);
    prop_assert!(problems.is_empty(), "{:?}\n{}", problems, xml);

    let mut names = Vec::new();
    names_in(doc.root(), &mut names);
    if !include_revisions {
        let left: Vec<_> = names
            .iter()
            .filter(|(n, _)| REVISION_NAMES.contains(&n.as_str()))
            .collect();
        prop_assert!(
            left.is_empty(),
            "revision markup remains: {:?}\n{}",
            left,
            xml
        );
        let expected = expected_after_accepting(source_body);
        prop_assert_eq!(&paragraphs, &expected, "\n{}", xml);
    }
    if !include_comments {
        prop_assert!(
            !names
                .iter()
                .any(|(n, _)| COMMENT_NAMES.contains(&n.as_str())),
            "comment markup remains\n{}",
            xml
        );
        prop_assert!(!files.contains_key("word/comments.xml"));
    }

    // 開始と終了の対応。コメントは参照も含めて 3 つ揃うか、すべて消える。
    let count = |name: &str, id: &str| names.iter().filter(|(n, i)| n == name && i == id).count();
    let ids: std::collections::BTreeSet<_> = names
        .iter()
        .filter(|(n, _)| COMMENT_NAMES.contains(&n.as_str()) || n.starts_with("bookmark"))
        .map(|(_, id)| id.clone())
        .collect();
    for id in &ids {
        let comment = [
            count("commentRangeStart", id),
            count("commentRangeEnd", id),
            count("commentReference", id),
        ];
        prop_assert!(
            comment == [0, 0, 0] || comment == [1, 1, 1],
            "comment {} {:?}\n{}",
            id,
            comment,
            xml
        );
        prop_assert_eq!(
            count("bookmarkStart", id),
            count("bookmarkEnd", id),
            "bookmark {}\n{}",
            id,
            xml
        );
    }
    // comments.xml のコメントと本文の参照は 1 対 1（本文のどこにも付いていないコメントを残さない）。
    if include_comments {
        let comments_xml = std::str::from_utf8(&files["word/comments.xml"]).unwrap();
        let comments_doc = DocxXmlDocument::parse(comments_xml)
            .map_err(|e| TestCaseError::fail(format!("{e:?}")))?;
        let mut comment_names = Vec::new();
        names_in(comments_doc.root(), &mut comment_names);
        let stored: std::collections::BTreeSet<_> = comment_names
            .iter()
            .filter(|(n, _)| n == "comment")
            .map(|(_, id)| id.clone())
            .collect();
        let referenced: std::collections::BTreeSet<_> = names
            .iter()
            .filter(|(n, _)| n == "commentReference")
            .map(|(_, id)| id.clone())
            .collect();
        prop_assert_eq!(
            &stored,
            &referenced,
            "comments.xml and the body references differ\n{}\n{}",
            comments_xml,
            xml
        );
    }
    let mut depth = 0_i32;
    for (name, kind) in &names {
        if name == "fldChar" {
            match kind.as_str() {
                "begin" => depth += 1,
                "end" => depth -= 1,
                _ => {}
            }
            prop_assert!(depth >= 0, "field end without begin\n{}", xml);
        }
    }
    prop_assert_eq!(depth, 0, "unterminated field\n{}", xml);
    Ok(())
}

fn transform(data: &[u8], comments: bool, revisions: bool) -> Vec<u8> {
    futures::executor::block_on(jade_docx_annotations::transform_docx_annotations(
        data, comments, revisions,
    ))
    .unwrap_or_else(|error| panic!("transform failed: {error}"))
}

proptest! {
    // 既定は 1024 件。深く探すときは `PROPTEST_CASES=50000 cargo test --release` のように増やす。
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES").ok().and_then(|cases| cases.parse().ok()).unwrap_or(1024),
        max_shrink_iters: 20_000,
        ..ProptestConfig::default()
    })]

    #[test]
    fn accepting_all_changes_matches_the_word_model(body in body(), spelling in spelling(), indented in any::<bool>()) {
        let source = render(&body, spelling, indented);
        for include_comments in [false, true] {
            for include_revisions in [false, true] {
                let output = transform(&source, include_comments, include_revisions);
                if include_comments && include_revisions {
                    prop_assert_eq!(&output, &source);
                }
                check(&body, &output, include_comments, include_revisions)?;
                let again = transform(&output, include_comments, include_revisions);
                prop_assert_eq!(parts(&again), parts(&output), "not idempotent");
            }
        }
    }
}
