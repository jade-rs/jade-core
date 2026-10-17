use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// プレーンテキスト以外の数式 (`m:oMath`) を保持するためのプレースホルダ。
///
/// 現時点では OMML の生 XML を保持し、後続のレンダリングパイプラインで
/// 構造化されたノードへ変換する予定。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct MathRun {
    /// `m:oMath` 要素全体の生 XML。
    pub raw_omml: String,
    /// 構造化された OMML 要素列。未解析の場合は空。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elements: Vec<MathElement>,
}

impl MathRun {
    /// 生 XML が空かどうかを判定するヘルパ。テストでの検証用。
    pub fn is_empty(&self) -> bool {
        self.raw_omml.trim().is_empty() && self.elements.is_empty()
    }

    /// 数式内のテキストを抽出し、プレビュー用に整形して返す。
    pub fn preview_text(&self, max_len: usize) -> Option<String> {
        preview_text_from_elements(&self.elements, max_len)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum MathElementKind {
    Math,
    MathParagraph,
    Run,
    Text,
    Fraction,
    Function,
    FunctionName,
    Numerator,
    Denominator,
    Radical,
    Base,
    Degree,
    Superscript,
    Subscript,
    PreSubSuper,
    SuperArgument,
    SubArgument,
    Nary,
    Delimiter,
    EquationArray,
    Limit,
    LimitLower,
    Matrix,
    MatrixRow,
    Box,
    Bar,
    GroupChar,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MathElement {
    pub kind: MathElementKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<MathElement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, String>,
}

/// ブロック数式 (`m:oMathPara`) を表す。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MathParagraph {
    /// `m:oMathPara` 全体の生 XML。
    pub raw_omml: String,
    /// 解析済み数式ノード列。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elements: Vec<MathElement>,
    /// Formatting of the enclosing `w:p`; it is not an extra editable paragraph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<Box<super::ParagraphProperties>>,
}

impl MathParagraph {
    /// 数式内のテキストを抽出し、プレビュー用に整形して返す。
    pub fn preview_text(&self, max_len: usize) -> Option<String> {
        preview_text_from_elements(&self.elements, max_len)
    }
}

fn preview_text_from_elements(elements: &[MathElement], max_len: usize) -> Option<String> {
    let mut buffer = String::new();
    collect_plain_text(elements, &mut buffer);
    if buffer.is_empty() {
        return None;
    }
    let normalized = buffer.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }
    Some(truncate_with_ellipsis(&normalized, max_len))
}

fn collect_plain_text(elements: &[MathElement], buffer: &mut String) {
    for element in elements {
        if let Some(tag) = element.tag.as_deref() {
            if tag.ends_with("Pr") {
                collect_plain_text(&element.children, buffer);
                continue;
            }
        }
        if let Some(text) = &element.text {
            append_token(buffer, text);
        }
        if !element.children.is_empty() {
            collect_plain_text(&element.children, buffer);
        }
    }
}

fn append_token(buffer: &mut String, token: &str) {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return;
    }
    if !buffer.is_empty() {
        buffer.push(' ');
    }
    buffer.push_str(trimmed);
}

fn truncate_with_ellipsis(text: &str, max_len: usize) -> String {
    if max_len == 0 {
        return String::new();
    }

    let mut truncated = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx == max_len {
            truncated.push('…');
            return truncated;
        }
        truncated.push(ch);
    }

    truncated
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_element(
        kind: MathElementKind,
        text: Option<&str>,
        children: Vec<MathElement>,
    ) -> MathElement {
        let tag_name = match &kind {
            MathElementKind::Math => "oMath",
            MathElementKind::MathParagraph => "oMathPara",
            MathElementKind::Run => "r",
            MathElementKind::Text => "t",
            MathElementKind::Fraction => "f",
            MathElementKind::Function => "func",
            MathElementKind::FunctionName => "fName",
            MathElementKind::Numerator => "num",
            MathElementKind::Denominator => "den",
            MathElementKind::Radical => "rad",
            MathElementKind::Base => "e",
            MathElementKind::Degree => "deg",
            MathElementKind::Superscript => "sSup",
            MathElementKind::Subscript => "sSub",
            MathElementKind::PreSubSuper => "sPre",
            MathElementKind::SuperArgument => "sup",
            MathElementKind::SubArgument => "sub",
            MathElementKind::Nary => "nary",
            MathElementKind::Delimiter => "d",
            MathElementKind::EquationArray => "eqArr",
            MathElementKind::Limit => "lim",
            MathElementKind::LimitLower => "limLow",
            MathElementKind::Matrix => "m",
            MathElementKind::MatrixRow => "mr",
            MathElementKind::Box => "box",
            MathElementKind::Bar => "bar",
            MathElementKind::GroupChar => "groupChr",
            MathElementKind::Unknown => "unknown",
        };
        MathElement {
            kind,
            tag: Some(tag_name.to_string()),
            text: text.map(|t| t.to_string()),
            children,
            attributes: BTreeMap::new(),
        }
    }

    #[test]
    fn math_run_preview_collapses_whitespace() {
        let elements = vec![
            make_element(
                MathElementKind::Run,
                None,
                vec![make_element(MathElementKind::Text, Some(" x "), Vec::new())],
            ),
            make_element(
                MathElementKind::Run,
                None,
                vec![make_element(MathElementKind::Text, Some(" + "), Vec::new())],
            ),
            make_element(
                MathElementKind::Run,
                None,
                vec![make_element(MathElementKind::Text, Some(" y"), Vec::new())],
            ),
        ];

        let run = MathRun {
            raw_omml: "<m:oMath>...</m:oMath>".to_string(),
            elements,
        };

        let preview = run.preview_text(16).expect("preview text");
        assert_eq!(preview, "x + y");
    }

    #[test]
    fn math_paragraph_preview_truncates_long_text() {
        let elements = vec![make_element(
            MathElementKind::Text,
            Some("abcdefghijklmnopqrstuvwxyz"),
            Vec::new(),
        )];

        let para = MathParagraph {
            properties: None,
            raw_omml: "<m:oMathPara>...</m:oMathPara>".to_string(),
            elements,
        };

        let preview = para.preview_text(5).expect("preview text");
        assert_eq!(preview, "abcde…");
    }

    #[test]
    fn math_preview_ignores_empty_content() {
        let para = MathParagraph {
            properties: None,
            raw_omml: "<m:oMathPara/>".to_string(),
            elements: Vec::new(),
        };
        assert!(para.preview_text(10).is_none());
    }
}
