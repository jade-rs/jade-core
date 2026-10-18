//! 段落や行を「挿入」「削除」に印を付ける。
//!
//! Word の変更履歴 (`w:ins` / `w:del`) と同じ形にする。こうしておくと、
//! 描く側は変更履歴を表示する既存の道をそのまま使える。合成のために
//! 独自の印を作らないのは、その道に乗せるため。
//!
//! 削除した文字は `w:t` ではなく `w:delText` に入れる決まりになっている
//! (ECMA-376 §17.3.3.7)。`w:t` のままだと本文として読まれてしまう。

use anyhow::{anyhow, Result};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

/// 変更履歴の作者。実在の人物ではないと分かる名前にする。
pub const AUTHOR: &str = "ellisii";
/// 変更履歴の日時。比較のたびに変わると差分が出るので固定する。
pub const DATE: &str = "1970-01-01T00:00:00Z";

/// `w:ins` / `w:del` に付ける属性。`id` は文書内で重複しなければよい。
fn revision_attrs(id: usize) -> String {
    format!(r#" w:id="{id}" w:author="{AUTHOR}" w:date="{DATE}""#)
}

/// 連番。1 つの文書の中で重ならなければよい。
#[derive(Debug, Default)]
pub struct RevisionIds(usize);

impl RevisionIds {
    pub fn next(&mut self) -> usize {
        self.0 += 1;
        self.0
    }
}

/// 段落の中の run を、まるごと挿入または削除として包む。
///
/// `w:pPr` の中の run プロパティ (`w:rPr`) は run ではないので触らない。
pub fn mark_runs(xml: &str, mode: Mode, ids: &mut RevisionIds) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    // `w:pPr` の中に入っている間は包まない。
    let mut in_para_props = 0usize;
    let mut depth_in_run = 0usize;

    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = name_of(&e);
                if name == "w:pPr" || name == "w:trPr" || name == "w:tcPr" {
                    in_para_props += 1;
                }
                if name == "w:r" && in_para_props == 0 && depth_in_run == 0 {
                    write_raw(&mut writer, &open_tag(mode, ids.next()))?;
                    depth_in_run = 1;
                    writer.write_event(Event::Start(e))?;
                    continue;
                }
                if depth_in_run > 0 {
                    depth_in_run += 1;
                }
                if mode == Mode::Delete && name == "w:t" && depth_in_run > 0 {
                    // 削除した文字は delText へ。属性 (xml:space) は残す。
                    let mut renamed = BytesStart::new("w:delText");
                    for attr in e.attributes().flatten() {
                        renamed.push_attribute(attr);
                    }
                    writer.write_event(Event::Start(renamed))?;
                    continue;
                }
                writer.write_event(Event::Start(e))?;
            }
            Ok(Event::End(e)) => {
                let name = name_of_end(&e);
                if name == "w:pPr" || name == "w:trPr" || name == "w:tcPr" {
                    in_para_props = in_para_props.saturating_sub(1);
                }
                if depth_in_run > 0 {
                    depth_in_run -= 1;
                    // 深さが 0 に戻ったところが、包み始めた run の終わり。
                    if depth_in_run == 0 && name == "w:r" {
                        writer.write_event(Event::End(e))?;
                        write_raw(&mut writer, close_tag(mode))?;
                        continue;
                    }
                }
                if mode == Mode::Delete && name == "w:t" && depth_in_run > 0 {
                    write_raw(&mut writer, "</w:delText>")?;
                    continue;
                }
                writer.write_event(Event::End(e))?;
            }
            Ok(event) => writer.write_event(event)?,
            Err(error) => return Err(anyhow!("段落の XML を読めません: {error}")),
        }
    }

    Ok(String::from_utf8(writer.into_inner())?)
}

/// 行そのものを挿入 / 削除として印を付ける (`w:trPr` の中に置く)。
///
/// run 側の印だけだと、Word も描く側も「行が増えた / 減った」とは受け取れない。
pub fn mark_row(xml: &str, mode: Mode, ids: &mut RevisionIds) -> Result<String> {
    let marked = mark_runs(xml, mode, ids)?;
    let marker = match mode {
        Mode::Insert => format!("<w:ins{}/>", revision_attrs(ids.next())),
        Mode::Delete => format!("<w:del{}/>", revision_attrs(ids.next())),
    };
    // `w:trPr` があればその先頭へ、無ければ `w:tr` の直後に作る。
    if let Some(at) = marked.find("<w:trPr>") {
        let at = at + "<w:trPr>".len();
        return Ok(format!("{}{marker}{}", &marked[..at], &marked[at..]));
    }
    let at = marked
        .find('>')
        .ok_or_else(|| anyhow!("w:tr の開始タグが見つかりません"))?
        + 1;
    Ok(format!(
        "{}<w:trPr>{marker}</w:trPr>{}",
        &marked[..at],
        &marked[at..]
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Insert,
    Delete,
}

fn open_tag(mode: Mode, id: usize) -> String {
    match mode {
        Mode::Insert => format!("<w:ins{}>", revision_attrs(id)),
        Mode::Delete => format!("<w:del{}>", revision_attrs(id)),
    }
}

fn close_tag(mode: Mode) -> &'static str {
    match mode {
        Mode::Insert => "</w:ins>",
        Mode::Delete => "</w:del>",
    }
}

fn name_of(e: &BytesStart<'_>) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

fn name_of_end(e: &quick_xml::events::BytesEnd<'_>) -> String {
    String::from_utf8_lossy(e.name().as_ref()).to_string()
}

/// 組み立て済みの XML をそのまま流し込む。
fn write_raw(writer: &mut Writer<Vec<u8>>, raw: &str) -> Result<()> {
    use std::io::Write;
    writer.get_mut().write_all(raw.as_bytes())?;
    Ok(())
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    #[test]
    fn 挿入は_run_を_w_ins_で包む() {
        let mut ids = RevisionIds::default();
        let out = mark_runs(
            r#"<w:p><w:r><w:t>あ</w:t></w:r></w:p>"#,
            Mode::Insert,
            &mut ids,
        )
        .unwrap();
        assert!(out.contains(r#"<w:ins w:id="1""#), "{out}");
        assert!(out.contains("<w:t>あ</w:t>"), "{out}");
        assert!(out.ends_with("</w:p>"), "{out}");
    }

    #[test]
    fn 削除は文字を_delText_にする() {
        let mut ids = RevisionIds::default();
        let out = mark_runs(
            r#"<w:p><w:r><w:t xml:space="preserve">あ </w:t></w:r></w:p>"#,
            Mode::Delete,
            &mut ids,
        )
        .unwrap();
        assert!(out.contains("<w:del "), "{out}");
        assert!(
            out.contains(r#"<w:delText xml:space="preserve">あ </w:delText>"#),
            "{out}"
        );
        assert!(!out.contains("<w:t "), "w:t が残っている: {out}");
    }

    #[test]
    fn 段落プロパティの中は包まない() {
        let mut ids = RevisionIds::default();
        let out = mark_runs(
            r#"<w:p><w:pPr><w:rPr><w:b/></w:rPr></w:pPr><w:r><w:t>あ</w:t></w:r></w:p>"#,
            Mode::Insert,
            &mut ids,
        )
        .unwrap();
        // 包むのは本文の run だけ。1 個だけ。
        assert_eq!(out.matches("<w:ins ").count(), 1, "{out}");
    }

    #[test]
    fn 行の印は_trPr_に入る() {
        let mut ids = RevisionIds::default();
        let out = mark_row(
            r#"<w:tr><w:tc><w:p><w:r><w:t>あ</w:t></w:r></w:p></w:tc></w:tr>"#,
            Mode::Insert,
            &mut ids,
        )
        .unwrap();
        assert!(out.contains("<w:trPr><w:ins "), "{out}");
    }

    #[test]
    fn 既にある_trPr_の先頭へ入れる() {
        let mut ids = RevisionIds::default();
        let out = mark_row(
            r#"<w:tr><w:trPr><w:trHeight w:val="100"/></w:trPr><w:tc><w:p/></w:tc></w:tr>"#,
            Mode::Delete,
            &mut ids,
        )
        .unwrap();
        assert!(out.contains("<w:trPr><w:del "), "{out}");
        assert!(out.contains("<w:trHeight"), "{out}");
        assert_eq!(out.matches("<w:trPr>").count(), 1, "{out}");
    }
}
