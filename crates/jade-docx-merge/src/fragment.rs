//! `word/document.xml` を、本文直下の要素ごとに切り出す。
//!
//! 合成では「新の文書のこの段落」「旧の文書のこの段落」を丸ごと持ってきて
//! 並べ直す。そのためには元の XML をそのまま取り出せる必要がある
//! (書式や図をこちらで組み直すと必ず落ちるものが出る)。
//!
//! 解析には quick-xml を使うが、**書き出しは元のバイト列の切り出し**にする。
//! 読んで書き直すと、名前空間の前置きや属性の並びが変わってしまう。

use anyhow::{anyhow, Result};
use quick_xml::events::Event;
use quick_xml::Reader;

/// 本文直下の要素 1 つ。
#[derive(Debug, Clone)]
pub struct Fragment {
    /// `w:p` / `w:tbl` など、前置き込みのタグ名。
    pub tag: String,
    /// 元の XML そのまま。
    pub xml: String,
}

impl Fragment {
    pub fn is_paragraph(&self) -> bool {
        self.tag == "w:p"
    }

    pub fn is_table(&self) -> bool {
        self.tag == "w:tbl"
    }

    /// この中にある `w:p` の数。表の中の段落も数える。
    pub fn paragraph_count(&self) -> usize {
        count_paragraphs(&self.xml)
    }
}

/// `<w:p>` の数。開始タグだけを数える (`<w:pPr>` などは数えない)。
fn count_paragraphs(xml: &str) -> usize {
    let bytes = xml.as_bytes();
    let mut count = 0usize;
    let mut i = 0usize;
    while let Some(at) = xml[i..].find("<w:p") {
        let at = i + at;
        let after = bytes.get(at + 4).copied();
        // `<w:p>` `<w:p ` `<w:p/>` だけ。`<w:pPr` `<w:pStyle` は違う。
        if matches!(after, Some(b'>') | Some(b' ') | Some(b'/')) {
            count += 1;
        }
        i = at + 4;
    }
    count
}

/// 本文の中身を切り出した結果。
///
/// 番号で引けるようにもしておく。差分は段落を文書順の通し番号で指してくるが、
/// その番号は表の中の段落にも振られていて、本文直下の何番目かとは一致しない。
pub struct Body {
    /// `<w:body>` の開始タグより前 (宣言や `<w:document ...>`)。
    pub prologue: String,
    /// 本文直下の要素。`w:sectPr` は含めない (末尾として別に持つ)。
    pub children: Vec<Fragment>,
    /// 最後の `w:sectPr` と `</w:body></w:document>`。
    pub epilogue: String,
}

impl Body {
    /// 表の通し番号 (1 始まり) から引く。
    pub fn table(&self, index: usize) -> Option<&Fragment> {
        self.children
            .iter()
            .filter(|child| child.is_table())
            .nth(index.saturating_sub(1))
    }
}

/// `<w:body>` の中身を、直下の要素ごとに切り出す。
pub fn split_body(xml: &str) -> Result<Body> {
    let bytes = xml.as_bytes();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut depth = 0usize;
    let mut in_body = false;
    let mut body_inner_start: Option<usize> = None;
    let mut children: Vec<Fragment> = Vec::new();
    // いま切り出している最中の要素。
    let mut current: Option<(String, usize)> = None;
    let mut sect_start: Option<usize> = None;
    let mut body_end: Option<usize> = None;

    loop {
        // 各イベントの開始位置は「読む前の位置」で押さえる。
        let before = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if !in_body && name == "w:body" {
                    in_body = true;
                    body_inner_start = Some(reader.buffer_position() as usize);
                    depth = 0;
                    continue;
                }
                if in_body {
                    if depth == 0 {
                        if name == "w:sectPr" {
                            sect_start = Some(before);
                        }
                        current = Some((name, before));
                    }
                    depth += 1;
                }
            }
            Ok(Event::End(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if in_body && name == "w:body" {
                    body_end = Some(before);
                    in_body = false;
                    continue;
                }
                if in_body {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        if let Some((tag, start)) = current.take() {
                            // 末尾の sectPr は本文の一部として扱わない。
                            if sect_start == Some(start) {
                                continue;
                            }
                            let end = reader.buffer_position() as usize;
                            children.push(Fragment {
                                tag,
                                xml: slice(bytes, start, end)?,
                            });
                        }
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                if in_body && depth == 0 {
                    if name == "w:sectPr" {
                        sect_start = Some(before);
                        continue;
                    }
                    let end = reader.buffer_position() as usize;
                    children.push(Fragment {
                        tag: name,
                        xml: slice(bytes, before, end)?,
                    });
                }
            }
            Ok(_) => {}
            Err(error) => return Err(anyhow!("document.xml を読めません: {error}")),
        }
    }

    let inner_start =
        body_inner_start.ok_or_else(|| anyhow!("document.xml に <w:body> がありません"))?;
    let body_end = body_end.ok_or_else(|| anyhow!("document.xml の </w:body> が見つかりません"))?;
    let tail_start = sect_start.unwrap_or(body_end);

    Ok(Body {
        prologue: slice(bytes, 0, inner_start)?,
        children,
        epilogue: slice(bytes, tail_start, bytes.len())?,
    })
}

fn slice(bytes: &[u8], start: usize, end: usize) -> Result<String> {
    let end = end.min(bytes.len());
    if start > end {
        return Err(anyhow!("XML の切り出し範囲がおかしい ({start}..{end})"));
    }
    Ok(String::from_utf8(bytes[start..end].to_vec())?)
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    const DOC: &str = concat!(
        r#"<?xml version="1.0"?><w:document xmlns:w="x"><w:body>"#,
        r#"<w:p><w:r><w:t>あ</w:t></w:r></w:p>"#,
        r#"<w:tbl><w:tr><w:tc><w:p><w:r><w:t>い</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        r#"<w:p><w:r><w:t>う</w:t></w:r></w:p>"#,
        r#"<w:sectPr><w:pgSz/></w:sectPr>"#,
        r#"</w:body></w:document>"#,
    );

    #[test]
    fn 本文直下だけを切り出す() {
        let body = split_body(DOC).unwrap();
        let tags: Vec<&str> = body.children.iter().map(|c| c.tag.as_str()).collect();
        // 表の中の段落は直下ではないので出てこない。
        assert_eq!(tags, ["w:p", "w:tbl", "w:p"]);
        assert!(body.children[0].xml.contains("あ"));
        assert!(body.children[1].xml.starts_with("<w:tbl>"));
        assert!(body.children[1].xml.ends_with("</w:tbl>"));
    }

    #[test]
    fn 前置きと末尾を分けて持つ() {
        let body = split_body(DOC).unwrap();
        assert!(body.prologue.contains("<w:body>"));
        assert!(body.epilogue.starts_with("<w:sectPr>"));
        assert!(body.epilogue.ends_with("</w:document>"));
    }

    #[test]
    fn 切り出したものを繋ぐと元に戻る() {
        let body = split_body(DOC).unwrap();
        let joined = format!(
            "{}{}{}",
            body.prologue,
            body.children
                .iter()
                .map(|c| c.xml.as_str())
                .collect::<String>(),
            body.epilogue,
        );
        assert_eq!(joined, DOC);
    }
}
