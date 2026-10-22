//! 書き換わった段落を、消えた文字と入った文字が両方見える形に組み直す。
//!
//! 差分は文字単位の内訳 (`segments`) を持っているので、それを順に run へ
//! 落とす。equal はそのまま、insert は `w:ins`、delete は `w:del` に入れる。
//!
//! **run ごとの書式は保てない。** 段落の書式 (`w:pPr`) と、その段落の最初の
//! run の書式 (`w:rPr`) は引き継ぐが、段落の途中で書体が変わっている場合は
//! そこまでは追えない。合成は「何が変わったか」を読むための見え方なので、
//! 文字の対応を優先している。

use anyhow::Result;
use docx_diff::{Segment, SegmentKind};

use crate::revision::RevisionIds;

/// 変わった段落を 1 つの `w:p` に組み直す。
///
/// `new_xml` は新しい側の段落。書式はここから取る。
pub fn rebuild_changed(
    new_xml: &str,
    segments: &[Segment],
    ids: &mut RevisionIds,
) -> Result<String> {
    let props = extract(new_xml, "w:pPr").unwrap_or_default();
    let run_props = first_run_props(new_xml).unwrap_or_default();

    let mut out = String::new();
    out.push_str(open_paragraph(new_xml));
    out.push_str(&props);
    for segment in segments {
        if segment.text.is_empty() {
            continue;
        }
        let text = escape(&segment.text);
        match segment.kind {
            SegmentKind::Equal => {
                out.push_str(&run(&run_props, &text, false));
            }
            SegmentKind::Insert => {
                let id = ids.next();
                out.push_str(&format!(
                    "<w:ins{}>{}</w:ins>",
                    ids.attrs(id),
                    run(&run_props, &text, false),
                ));
            }
            SegmentKind::Delete => {
                let id = ids.next();
                out.push_str(&format!(
                    "<w:del{}>{}</w:del>",
                    ids.attrs(id),
                    run(&run_props, &text, true),
                ));
            }
        }
    }
    out.push_str("</w:p>");
    Ok(out)
}

fn run(run_props: &str, text: &str, deleted: bool) -> String {
    let tag = if deleted { "w:delText" } else { "w:t" };
    format!(r#"<w:r>{run_props}<{tag} xml:space="preserve">{text}</{tag}></w:r>"#)
}

/// `<w:p ...>` の開始タグをそのまま返す (`w14:paraId` などを残すため)。
fn open_paragraph(xml: &str) -> &str {
    match xml.find('>') {
        Some(at) => &xml[..=at],
        None => "<w:p>",
    }
}

/// 最初の run の `w:rPr` を取り出す。段落全体の書体をこれで代表させる。
fn first_run_props(xml: &str) -> Option<String> {
    // `w:pPr` の中の `w:rPr` は段落記号の書式なので飛ばす。
    let after_props = match xml.find("</w:pPr>") {
        Some(at) => &xml[at..],
        None => xml,
    };
    let run_start = after_props
        .find("<w:r>")
        .or_else(|| after_props.find("<w:r "))?;
    extract(&after_props[run_start..], "w:rPr")
}

/// `<tag ...>...</tag>` あるいは `<tag ... />` を丸ごと取り出す。
fn extract(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let start = xml.find(&open)?;
    let rest = &xml[start..];
    let head_end = rest.find('>')?;
    if rest[..head_end].ends_with('/') {
        return Some(rest[..=head_end].to_string());
    }
    let close = format!("</{tag}>");
    let end = rest.find(&close)? + close.len();
    Some(rest[..end].to_string())
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    fn seg(kind: SegmentKind, text: &str) -> Segment {
        Segment {
            kind,
            text: text.to_string(),
        }
    }

    #[test]
    fn 消えた文字と入った文字が両方残る() {
        let mut ids = RevisionIds::default();
        let out = rebuild_changed(
            r#"<w:p w14:paraId="1"><w:pPr><w:jc w:val="both"/></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t>新</w:t></w:r></w:p>"#,
            &[
                seg(SegmentKind::Equal, "共通"),
                seg(SegmentKind::Delete, "消えた"),
                seg(SegmentKind::Insert, "入った"),
            ],
            &mut ids,
        )
        .unwrap();

        assert!(out.starts_with(r#"<w:p w14:paraId="1">"#), "{out}");
        assert!(
            out.contains(r#"<w:jc w:val="both"/>"#),
            "段落書式が落ちた: {out}"
        );
        assert!(
            out.contains("<w:t xml:space=\"preserve\">共通</w:t>"),
            "{out}"
        );
        assert!(
            out.contains("<w:delText xml:space=\"preserve\">消えた</w:delText>"),
            "{out}"
        );
        assert!(out.contains("<w:ins "), "{out}");
        // 消えた文字は w:t には出さない (本文として読まれてしまう)。
        assert!(
            !out.contains(r#"<w:t xml:space="preserve">消えた"#),
            "{out}"
        );
        // run の書式は引き継ぐ。
        assert_eq!(out.matches("<w:b/>").count(), 3, "{out}");
    }

    #[test]
    fn 変更は渡した名前と日時で記録する() {
        let mut ids = RevisionIds::labelled("Ann & Co", "2026-10-22T09:00:00Z");
        let out = rebuild_changed(
            r#"<w:p><w:r><w:t>x</w:t></w:r></w:p>"#,
            &[
                seg(SegmentKind::Delete, "old"),
                seg(SegmentKind::Insert, "new"),
            ],
            &mut ids,
        )
        .unwrap();
        assert_eq!(
            out.matches(r#"w:author="Ann &amp; Co""#).count(),
            2,
            "{out}"
        );
        assert!(out.contains(r#"w:date="2026-10-22T09:00:00Z""#), "{out}");
    }

    #[test]
    fn 記号を含む文字を壊さない() {
        let mut ids = RevisionIds::default();
        let out = rebuild_changed(
            r#"<w:p><w:r><w:t>x</w:t></w:r></w:p>"#,
            &[seg(SegmentKind::Insert, "a<b>&c")],
            &mut ids,
        )
        .unwrap();
        assert!(out.contains("a&lt;b&gt;&amp;c"), "{out}");
    }

    #[test]
    fn 段落記号の書式は_run_の書式として使わない() {
        let mut ids = RevisionIds::default();
        let out = rebuild_changed(
            r#"<w:p><w:pPr><w:rPr><w:i/></w:rPr></w:pPr><w:r><w:rPr><w:b/></w:rPr><w:t>新</w:t></w:r></w:p>"#,
            &[seg(SegmentKind::Equal, "あ")],
            &mut ids,
        )
        .unwrap();
        // 引き継ぐのは本文 run の書式 (b) であって、段落記号の書式 (i) ではない。
        assert!(out.contains("<w:b/>"), "{out}");
        assert_eq!(out.matches("<w:i/>").count(), 1, "pPr のぶんだけ: {out}");
    }
}
