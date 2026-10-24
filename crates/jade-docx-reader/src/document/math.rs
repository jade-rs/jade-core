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
    /// `m:acc`: an accent over its base.
    Accent,
    /// `m:sSubSup`: a base with a subscript and a superscript.
    SubSup,
    /// `m:limUpp`: a limit over its base.
    LimitUpper,
    /// `m:borderBox`: a base in a box of rules and strikes.
    BorderBox,
    /// `m:phant`: a base kept for its size, shown or not.
    Phantom,
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
    /// The element's namespace prefix when it is not the math one (`w` for a
    /// `w:rPr` inside `m:r`), so the tree can be written back as it was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
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

impl MathElementKind {
    /// The OMML local name an element of this kind is written with.
    pub fn omml_tag(&self) -> &'static str {
        match self {
            Self::Math => "oMath",
            Self::MathParagraph => "oMathPara",
            Self::Run => "r",
            Self::Text => "t",
            Self::Fraction => "f",
            Self::Function => "func",
            Self::FunctionName => "fName",
            Self::Numerator => "num",
            Self::Denominator => "den",
            Self::Radical => "rad",
            Self::Base => "e",
            Self::Degree => "deg",
            Self::Superscript => "sSup",
            Self::Subscript => "sSub",
            Self::PreSubSuper => "sPre",
            Self::SuperArgument => "sup",
            Self::SubArgument => "sub",
            Self::Nary => "nary",
            Self::Delimiter => "d",
            Self::EquationArray => "eqArr",
            Self::Limit => "lim",
            Self::LimitLower => "limLow",
            Self::Matrix => "m",
            Self::MatrixRow => "mr",
            Self::Box => "box",
            Self::Bar => "bar",
            Self::GroupChar => "groupChr",
            Self::Accent => "acc",
            Self::SubSup => "sSubSup",
            Self::LimitUpper => "limUpp",
            Self::BorderBox => "borderBox",
            Self::Phantom => "phant",
            Self::Unknown => "r",
        }
    }

    /// The kind an OMML local name reads as.
    pub fn from_omml_tag(local: &str) -> Self {
        match local {
            "oMath" => Self::Math,
            "oMathPara" => Self::MathParagraph,
            "r" => Self::Run,
            "t" => Self::Text,
            "f" => Self::Fraction,
            "num" => Self::Numerator,
            "den" => Self::Denominator,
            "rad" => Self::Radical,
            "deg" => Self::Degree,
            "e" => Self::Base,
            "sSup" => Self::Superscript,
            "sSub" => Self::Subscript,
            "sSubSup" => Self::SubSup,
            "sPre" => Self::PreSubSuper,
            "sup" => Self::SuperArgument,
            "sub" => Self::SubArgument,
            "nary" => Self::Nary,
            "d" => Self::Delimiter,
            "box" => Self::Box,
            "bar" => Self::Bar,
            "groupChr" => Self::GroupChar,
            "eqArr" => Self::EquationArray,
            "lim" => Self::Limit,
            "limLow" => Self::LimitLower,
            "limUpp" => Self::LimitUpper,
            "m" => Self::Matrix,
            "mr" => Self::MatrixRow,
            "func" => Self::Function,
            "fName" => Self::FunctionName,
            "acc" => Self::Accent,
            "borderBox" => Self::BorderBox,
            "phant" => Self::Phantom,
            _ => Self::Unknown,
        }
    }
}

impl MathElement {
    /// A math element of `kind` with these children.
    pub fn new(kind: MathElementKind, children: Vec<MathElement>) -> Self {
        Self {
            tag: Some(kind.omml_tag().to_string()),
            kind,
            children,
            ..Self::default()
        }
    }

    /// An element named by its local `tag`, its kind read from the name.
    pub fn tagged(tag: &str, children: Vec<MathElement>) -> Self {
        Self {
            kind: MathElementKind::from_omml_tag(tag),
            tag: Some(tag.to_string()),
            children,
            ..Self::default()
        }
    }

    /// A property element `<m:{tag} m:val="{value}"/>`.
    pub fn property(tag: &str, value: &str) -> Self {
        let mut element = Self::tagged(tag, Vec::new());
        element.kind = MathElementKind::Unknown;
        element
            .attributes
            .insert("val".to_string(), value.to_string());
        element
    }

    /// An `m:t` holding `text`.
    pub fn text_node(text: &str) -> Self {
        Self {
            kind: MathElementKind::Text,
            tag: Some("t".to_string()),
            text: Some(text.to_string()),
            ..Self::default()
        }
    }

    /// The local name, from the tag or else the kind.
    pub fn local_name(&self) -> &str {
        match self.tag.as_deref() {
            Some(tag) => tag.rsplit_once(':').map_or(tag, |(_, local)| local),
            None => self.kind.omml_tag(),
        }
    }

    /// The qualified name this element is written with.
    pub fn qualified_name(&self) -> String {
        if let Some(tag) = self.tag.as_deref() {
            if tag.contains(':') {
                return tag.to_string();
            }
        }
        match self.namespace.as_deref() {
            Some("") => self.local_name().to_string(),
            Some(prefix) => format!("{prefix}:{}", self.local_name()),
            None => format!("m:{}", self.local_name()),
        }
    }

    fn prefix(&self) -> String {
        let name = self.qualified_name();
        name.split_once(':')
            .map(|(prefix, _)| prefix.to_string())
            .unwrap_or_default()
    }

    /// Writes the element as OMML. `root` adds the math and Word namespace
    /// declarations the fragment needs to stand on its own.
    pub fn write_xml(&self, out: &mut String, root: bool) {
        let name = self.qualified_name();
        let prefix = self.prefix();
        out.push('<');
        out.push_str(&name);
        let mut declared_m = false;
        let mut declared_w = false;
        for (key, value) in &self.attributes {
            out.push(' ');
            let namespace_uri = value.starts_with("http://") || value.starts_with("urn:");
            if key == "xmlns" {
                out.push_str("xmlns");
            } else if namespace_uri && !key.contains(':') && key.len() <= 8 {
                declared_m |= key == "m";
                declared_w |= key == "w";
                out.push_str("xmlns:");
                out.push_str(key);
            } else if key == "space" {
                out.push_str("xml:");
                out.push_str(key);
            } else if key.contains(':') || prefix.is_empty() {
                out.push_str(key);
            } else {
                out.push_str(&prefix);
                out.push(':');
                out.push_str(key);
            }
            out.push_str("=\"");
            escape_into(value, out, true);
            out.push('"');
        }
        if root {
            if !declared_m {
                out.push_str(
                    r#" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math""#,
                );
            }
            if !declared_w {
                out.push_str(
                    r#" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#,
                );
            }
        }
        let text = self.text.as_deref().filter(|text| !text.is_empty());
        if let Some(text) = text {
            if self.kind == MathElementKind::Text
                && !self.attributes.contains_key("space")
                && (text.starts_with(char::is_whitespace) || text.ends_with(char::is_whitespace))
            {
                out.push_str(r#" xml:space="preserve""#);
            }
        }
        if text.is_none() && self.children.is_empty() {
            out.push_str("/>");
            return;
        }
        out.push('>');
        if let Some(text) = text {
            escape_into(text, out, false);
        }
        for child in &self.children {
            child.write_xml(out, false);
        }
        out.push_str("</");
        out.push_str(&name);
        out.push('>');
    }

    /// The element as a standalone OMML fragment.
    pub fn to_omml(&self) -> String {
        let mut out = String::new();
        self.write_xml(&mut out, true);
        out
    }
}

fn escape_into(text: &str, out: &mut String, attribute: bool) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

/// Writes parsed math elements back as OMML: the first root is written with
/// the namespace declarations, so an `m:oMath` or `m:oMathPara` read from a
/// document comes back as a fragment that parses to the same tree.
pub fn math_elements_to_omml(elements: &[MathElement]) -> String {
    let mut out = String::new();
    for (index, element) in elements.iter().enumerate() {
        element.write_xml(&mut out, index == 0);
    }
    out
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
            MathElementKind::Accent => "acc",
            MathElementKind::SubSup => "sSubSup",
            MathElementKind::LimitUpper => "limUpp",
            MathElementKind::BorderBox => "borderBox",
            MathElementKind::Phantom => "phant",
            MathElementKind::Unknown => "unknown",
        };
        MathElement {
            kind,
            tag: Some(tag_name.to_string()),
            text: text.map(|t| t.to_string()),
            children,
            attributes: BTreeMap::new(),
            namespace: None,
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

    const EVERY_STRUCTURE: &str = concat!(
        r#"<m:oMathPara xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
        r#"<m:oMathParaPr><m:jc m:val="centerGroup"/></m:oMathParaPr><m:oMath>"#,
        r#"<m:acc><m:accPr><m:chr m:val="̂"/><m:ctrlPr><w:rPr><w:rFonts w:ascii="Cambria Math" w:hAnsi="Cambria Math"/><w:i/></w:rPr></m:ctrlPr></m:accPr><m:e><m:r><m:t>a</m:t></m:r></m:e></m:acc>"#,
        r#"<m:bar><m:barPr><m:pos m:val="top"/></m:barPr><m:e><m:r><m:t>b</m:t></m:r></m:e></m:bar>"#,
        r#"<m:borderBox><m:borderBoxPr><m:hideTop m:val="1"/><m:strikeBLTR m:val="1"/></m:borderBoxPr><m:e><m:r><m:t>c</m:t></m:r></m:e></m:borderBox>"#,
        r#"<m:box><m:boxPr><m:opEmu m:val="1"/></m:boxPr><m:e><m:r><m:t>==</m:t></m:r></m:e></m:box>"#,
        r#"<m:d><m:dPr><m:begChr m:val="["/><m:sepChr m:val="|"/><m:endChr m:val=""/><m:grow m:val="1"/></m:dPr><m:e><m:r><m:t>x</m:t></m:r></m:e><m:e><m:r><m:t>y</m:t></m:r></m:e></m:d>"#,
        r#"<m:eqArr><m:eqArrPr><m:baseJc m:val="top"/></m:eqArrPr><m:e><m:r><m:t>x&amp;=1</m:t></m:r></m:e><m:e><m:r><m:t>y&amp;=2</m:t></m:r></m:e></m:eqArr>"#,
        r#"<m:f><m:fPr><m:type m:val="skw"/></m:fPr><m:num><m:r><m:t>1</m:t></m:r></m:num><m:den><m:r><m:t>2</m:t></m:r></m:den></m:f>"#,
        r#"<m:func><m:fName><m:r><m:rPr><m:sty m:val="p"/></m:rPr><m:t>sin</m:t></m:r></m:fName><m:e><m:r><m:t>θ</m:t></m:r></m:e></m:func>"#,
        r#"<m:groupChr><m:groupChrPr><m:chr m:val="⏞"/><m:pos m:val="top"/><m:vertJc m:val="bot"/></m:groupChrPr><m:e><m:r><m:t>a+b</m:t></m:r></m:e></m:groupChr>"#,
        r#"<m:limLow><m:e><m:r><m:rPr><m:sty m:val="p"/></m:rPr><m:t>lim</m:t></m:r></m:e><m:lim><m:r><m:t>n→∞</m:t></m:r></m:lim></m:limLow>"#,
        r#"<m:limUpp><m:e><m:r><m:t>x</m:t></m:r></m:e><m:lim><m:r><m:t>def</m:t></m:r></m:lim></m:limUpp>"#,
        r#"<m:m><m:mPr><m:mcs><m:mc><m:mcPr><m:count m:val="2"/><m:mcJc m:val="center"/></m:mcPr></m:mc></m:mcs></m:mPr><m:mr><m:e><m:r><m:t>1</m:t></m:r></m:e><m:e><m:r><m:t>0</m:t></m:r></m:e></m:mr><m:mr><m:e/><m:e><m:r><m:t>1</m:t></m:r></m:e></m:mr></m:m>"#,
        r#"<m:nary><m:naryPr><m:chr m:val="∑"/><m:limLoc m:val="undOvr"/></m:naryPr><m:sub><m:r><m:t>i=1</m:t></m:r></m:sub><m:sup><m:r><m:t>n</m:t></m:r></m:sup><m:e><m:r><m:t>i</m:t></m:r></m:e></m:nary>"#,
        r#"<m:phant><m:phantPr><m:show m:val="0"/><m:zeroWid m:val="1"/></m:phantPr><m:e><m:r><m:t>x</m:t></m:r></m:e></m:phant>"#,
        r#"<m:rad><m:radPr><m:degHide m:val="1"/></m:radPr><m:deg/><m:e><m:r><m:t>2</m:t></m:r></m:e></m:rad>"#,
        r#"<m:sPre><m:sub><m:r><m:t>1</m:t></m:r></m:sub><m:sup><m:r><m:t>2</m:t></m:r></m:sup><m:e><m:r><m:t>X</m:t></m:r></m:e></m:sPre>"#,
        r#"<m:sSub><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sub><m:r><m:t>i</m:t></m:r></m:sub></m:sSub>"#,
        r#"<m:sSubSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sub><m:r><m:t>i</m:t></m:r></m:sub><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSubSup>"#,
        r#"<m:sSup><m:e><m:r><m:t>e</m:t></m:r></m:e><m:sup><m:r><m:t>x</m:t></m:r></m:sup></m:sSup>"#,
        r#"<m:r><m:rPr><m:nor/><m:brk m:alnAt="2"/></m:rPr><w:rPr><w:rFonts w:ascii="Cambria Math"/><w:color w:val="FF0000"/></w:rPr><m:t xml:space="preserve"> and </m:t></m:r>"#,
        r#"</m:oMath></m:oMathPara>"#
    );

    #[test]
    fn every_omml_structure_writes_back_to_the_tree_it_was_read_from() {
        let first = crate::parser::math::parse_math_elements(EVERY_STRUCTURE).unwrap();
        let written = math_elements_to_omml(&first);
        let second = crate::parser::math::parse_math_elements(&written).unwrap();
        assert_eq!(first, second, "{written}");
        // Word elements keep their own prefix; math ones get theirs.
        assert!(written.contains(
            "<w:rPr><w:rFonts w:ascii=\"Cambria Math\" w:hAnsi=\"Cambria Math\"/><w:i/></w:rPr>"
        ));
        assert!(written.contains("<m:rPr><m:nor/><m:brk m:alnAt=\"2\"/></m:rPr>"));
        assert!(written.contains("<m:t xml:space=\"preserve\"> and </m:t>"));
        assert!(written.contains("<m:t>x&amp;=1</m:t>"));
        assert!(written.contains("<m:endChr m:val=\"\"/>"));
        // Every structure reads as its own kind.
        let mut kinds = Vec::new();
        fn walk(element: &MathElement, kinds: &mut Vec<MathElementKind>) {
            kinds.push(element.kind.clone());
            for child in &element.children {
                walk(child, kinds);
            }
        }
        walk(&second[0], &mut kinds);
        for kind in [
            MathElementKind::Accent,
            MathElementKind::Bar,
            MathElementKind::BorderBox,
            MathElementKind::Box,
            MathElementKind::Delimiter,
            MathElementKind::EquationArray,
            MathElementKind::Fraction,
            MathElementKind::Function,
            MathElementKind::GroupChar,
            MathElementKind::LimitLower,
            MathElementKind::LimitUpper,
            MathElementKind::Matrix,
            MathElementKind::MatrixRow,
            MathElementKind::Nary,
            MathElementKind::Phantom,
            MathElementKind::Radical,
            MathElementKind::PreSubSuper,
            MathElementKind::Subscript,
            MathElementKind::SubSup,
            MathElementKind::Superscript,
        ] {
            assert!(kinds.contains(&kind), "{kind:?}");
        }
    }

    #[test]
    fn constructed_elements_write_with_math_names_and_declarations() {
        let fraction = MathElement::new(
            MathElementKind::Fraction,
            vec![
                MathElement::new(
                    MathElementKind::Numerator,
                    vec![MathElement::new(
                        MathElementKind::Run,
                        vec![MathElement::text_node("a<b")],
                    )],
                ),
                MathElement::new(MathElementKind::Denominator, Vec::new()),
            ],
        );
        let math = MathElement::new(MathElementKind::Math, vec![fraction]);
        assert_eq!(
            math.to_omml(),
            concat!(
                r#"<m:oMath xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
                r#"<m:f><m:num><m:r><m:t>a&lt;b</m:t></m:r></m:num><m:den/></m:f></m:oMath>"#
            )
        );
    }
}
