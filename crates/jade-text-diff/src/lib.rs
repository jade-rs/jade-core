//! 文字列の内訳を、形態素で切ってから出す。
//!
//! これまでは 1 文字ずつ突き合わせていた。日本語には単語の切れ目が無いので
//! 素直な選択ではあるが、読むほうには辛い。「誠意をもって」→「誠実に」の
//! ような書き換えが
//!
//! ```text
//! 誠[意をもって]{実に}
//! ```
//!
//! ではなく `誠` を残したまま `意` `を` `も` … と虫食いに出る。**変わったのは
//! 語であって文字ではない**ので、語で切ってから比べる。
//!
//! 切るのは [delarocha](https://github.com/bokuweb/delarocha)。辞書は実行時に
//! 読む外部ファイルなので、**無ければ書記素クラスタ単位に戻る**。結合文字や
//! ZWJ で繋いだ絵文字を途中で切らず、辞書を置き忘れても比較そのものは動く。

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use delarocha::{VibratoSystemDictionary, VibratoSystemTokenizer};
use diff_align::Step;
use diff_support::trimmed_lcs_diff;
use docx_diff::{Segment, SegmentKind};
use quick_xml::events::Event as XmlEvent;
use quick_xml::Reader as XmlReader;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;
use zip::ZipArchive;

/// 形式に依らない内訳の 1 つ。
#[derive(Debug, Clone)]
pub struct Unit {
    pub kind: SegmentKind,
    pub text: String,
}

/// 文字列を切って比べる人。
///
/// 辞書を読めていれば語で、読めていなければ書記素クラスタ単位で切る。
pub struct Segmenter {
    tokenizer: Option<VibratoSystemTokenizer>,
}

impl std::fmt::Debug for Segmenter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Segmenter")
            .field("形態素", &self.tokenizer.is_some())
            .finish()
    }
}

impl Segmenter {
    /// 辞書を読んで用意する。`system.dic` と `system.dic.zst` のどちらでもよい。
    pub fn from_dictionary(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let dictionary = VibratoSystemDictionary::from_path(path)
            .with_context(|| format!("辞書を読めません: {}", path.display()))?;
        let tokenizer = dictionary
            .into_tokenizer()
            // 空白は語にしない。docx の本文には全角空白が入ることがあり、
            // それ自体を「変わった」と出しても読む助けにならない。
            .ignore_space(true)
            .context("辞書の設定に失敗しました")?
            // 記号や英数が長く続くときに、1 語としてまとめる上限。
            .max_grouping_len(24);
        Ok(Self {
            tokenizer: Some(tokenizer),
        })
    }

    /// 辞書なし。ユーザーが 1 文字と認識する書記素クラスタ単位で切る。
    ///
    /// API 互換性のため名前は `char_only` のままにしている。
    pub fn char_only() -> Self {
        Self { tokenizer: None }
    }

    /// 辞書を読めたか。読めていなければ書記素クラスタ単位に落ちている。
    pub fn has_dictionary(&self) -> bool {
        self.tokenizer.is_some()
    }

    /// 2 つの文字列の内訳を、形式に依らない形で出す。
    ///
    /// docx / pdf / pptx でそれぞれ別の型を使っているので、変換はそちらに任せる。
    pub fn units(&self, old: &str, new: &str) -> Vec<Unit> {
        self.segments(old, new)
            .into_iter()
            .map(|segment| Unit {
                kind: segment.kind,
                text: segment.text,
            })
            .collect()
    }

    /// 2 つの文字列の内訳を出す。
    pub fn segments(&self, old: &str, new: &str) -> Vec<Segment> {
        let old_units = self.split(old);
        let new_units = self.split(new);
        let steps = trimmed_lcs_diff(&old_units, &new_units);

        let mut out: Vec<Segment> = Vec::new();
        for step in steps {
            let (kind, text) = match step {
                Step::Equal(o, _) => (SegmentKind::Equal, old_units[o].as_str()),
                Step::Removed(o) => (SegmentKind::Delete, old_units[o].as_str()),
                Step::Added(n) => (SegmentKind::Insert, new_units[n].as_str()),
            };
            match out.last_mut() {
                Some(last) if last.kind == kind => last.text.push_str(text),
                _ => out.push(Segment {
                    kind,
                    text: text.to_string(),
                }),
            }
        }
        out
    }

    /// 比べる単位に切る。
    ///
    /// 語の間の空白も落とさずに拾う。落とすと、繋ぎ直したときに元の文字列に
    /// 戻らなくなり、内訳のバイト位置が本文とずれる (装飾がその位置で塗られる)。
    fn split(&self, text: &str) -> Vec<String> {
        let Some(tokenizer) = &self.tokenizer else {
            return grapheme_units(text);
        };
        let Ok(tokens) = tokenizer.tokenize(text) else {
            return grapheme_units(text);
        };

        let mut out: Vec<String> = Vec::new();
        let mut at = 0usize;
        for token in &tokens {
            let range = token.range_byte();
            if range.start > at {
                // 語と語のあいだ (空白など)。1 つの単位として挟む。
                out.push(text[at..range.start].to_string());
            }
            if range.end > range.start {
                out.push(text[range.start..range.end].to_string());
            }
            at = range.end.max(at);
        }
        if at < text.len() {
            out.push(text[at..].to_string());
        }
        merge_at_grapheme_boundaries(text, out)
    }
}

fn grapheme_units(text: &str) -> Vec<String> {
    text.graphemes(true).map(str::to_owned).collect()
}

/// 形態素解析器が結合文字や ZWJ 絵文字の途中に境界を返しても、隣の単位と
/// 結合して表示上の 1 文字を壊さないようにする。
fn merge_at_grapheme_boundaries(text: &str, units: Vec<String>) -> Vec<String> {
    let boundaries: BTreeSet<usize> = text
        .grapheme_indices(true)
        .map(|(start, _)| start)
        .chain(std::iter::once(text.len()))
        .collect();
    let mut out = Vec::with_capacity(units.len());
    let mut pending = String::new();
    let mut at = 0;

    for unit in units {
        at += unit.len();
        pending.push_str(&unit);
        if boundaries.contains(&at) {
            out.push(std::mem::take(&mut pending));
        }
    }
    debug_assert_eq!(at, text.len());
    debug_assert!(pending.is_empty());
    out
}

/// 差分の中の内訳を、語で切り直したものに入れ替える。
///
/// 比較そのものは jade 側で済んでいる。ここで直すのは「どこが変わったか」の
/// 見せ方に加え、Unicode の NFC/NFD だけが違う段落の誤検出も落とす。
/// 対応付けそのものには触らない。
pub fn resegment_docx(diff: &mut docx_diff::DocumentDiff, segmenter: &Segmenter) {
    fn walk(blocks: &mut [docx_diff::BlockChange], segmenter: &Segmenter) {
        for block in blocks {
            match block {
                docx_diff::BlockChange::Paragraph(change) => {
                    if change.kind != docx_diff::ChangeKind::Changed {
                        continue;
                    }
                    // 内訳は番号を含まない本文に対して出す。装飾もそちらの
                    // 位置で塗るため。
                    let old = change.old_body_text.as_deref().unwrap_or_default();
                    let new = change.new_body_text.as_deref().unwrap_or_default();
                    change.segments = segmenter.segments(old, new);
                }
                docx_diff::BlockChange::Table(change) => {
                    for row in &mut change.rows {
                        for cell in &mut row.cells {
                            walk(&mut cell.blocks, segmenter);
                        }
                    }
                }
            }
        }
    }
    suppress_canonical_equivalent_changes(&mut diff.blocks);
    apply_docx_format_changes(diff);
    walk(&mut diff.blocks, segmenter);
    diff.summary = summarize_docx(&diff.blocks);
}

fn suppress_canonical_equivalent_changes(blocks: &mut [docx_diff::BlockChange]) {
    for block in blocks {
        match block {
            docx_diff::BlockChange::Paragraph(change) => {
                if change.kind != docx_diff::ChangeKind::Changed {
                    continue;
                }
                let Some(old) = change.old_text.as_deref() else {
                    continue;
                };
                let Some(new) = change.new_text.as_deref() else {
                    continue;
                };
                if comparison_text(old) == comparison_text(new) {
                    change.kind = if change.moved && docx_diff::normalize(new).chars().count() >= 2
                    {
                        docx_diff::ChangeKind::Moved
                    } else {
                        docx_diff::ChangeKind::Unchanged
                    };
                    change.segments.clear();
                }
            }
            docx_diff::BlockChange::Table(table) => {
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        suppress_canonical_equivalent_changes(&mut cell.blocks);
                        if cell.kind == docx_diff::ChangeKind::Changed
                            && cell
                                .blocks
                                .iter()
                                .all(|block| block.kind() == docx_diff::ChangeKind::Unchanged)
                        {
                            cell.kind = docx_diff::ChangeKind::Unchanged;
                        }
                    }
                    if row.kind == docx_diff::ChangeKind::Changed
                        && row
                            .cells
                            .iter()
                            .all(|cell| cell.kind == docx_diff::ChangeKind::Unchanged)
                    {
                        row.kind = if row.moved {
                            docx_diff::ChangeKind::Moved
                        } else {
                            docx_diff::ChangeKind::Unchanged
                        };
                    }
                }
                if table.kind == docx_diff::ChangeKind::Changed
                    && table
                        .rows
                        .iter()
                        .all(|row| row.kind == docx_diff::ChangeKind::Unchanged)
                {
                    table.kind = if table.moved {
                        docx_diff::ChangeKind::Moved
                    } else {
                        docx_diff::ChangeKind::Unchanged
                    };
                }
            }
        }
    }
}

fn comparison_text(text: &str) -> String {
    text.chars()
        .filter(|character| *character != '\u{ad}')
        .nfc()
        .collect()
}

#[derive(Debug, Clone, Default)]
struct DocxNode {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<DocxNode>,
    text: String,
}

impl DocxNode {
    fn attr(&self, name: &str) -> Option<&str> {
        let wanted = name.rsplit(':').next().unwrap_or(name);
        self.attrs
            .iter()
            .find(|(key, _)| key.rsplit(':').next().unwrap_or(key) == wanted)
            .map(|(_, value)| value.as_str())
    }

    fn child(&self, name: &str) -> Option<&DocxNode> {
        self.children.iter().find(|child| child.name == name)
    }

    fn visit<'a>(&'a self, visitor: &mut impl FnMut(&'a DocxNode)) {
        visitor(self);
        for child in &self.children {
            child.visit(visitor);
        }
    }
}

fn apply_docx_format_changes(diff: &mut docx_diff::DocumentDiff) {
    let (Ok(old_formats), Ok(new_formats)) = (
        docx_paragraph_format_signatures(Path::new(&diff.old_file)),
        docx_paragraph_format_signatures(Path::new(&diff.new_file)),
    ) else {
        return;
    };

    fn walk(
        blocks: &mut [docx_diff::BlockChange],
        old_formats: &[String],
        new_formats: &[String],
    ) -> bool {
        let mut changed = false;
        for block in blocks {
            match block {
                docx_diff::BlockChange::Paragraph(change) => {
                    let same_text = change
                        .old_body_text
                        .as_deref()
                        .zip(change.new_body_text.as_deref())
                        .is_some_and(|(old, new)| comparison_text(old) == comparison_text(new));
                    let format_changed = change
                        .old_index
                        .zip(change.new_index)
                        .and_then(|(old, new)| {
                            old_formats
                                .get(old.saturating_sub(1))
                                .zip(new_formats.get(new.saturating_sub(1)))
                        })
                        .is_some_and(|(old, new)| old != new);
                    if same_text && format_changed {
                        change.kind = docx_diff::ChangeKind::Changed;
                        changed = true;
                    }
                }
                docx_diff::BlockChange::Table(table) => {
                    let mut table_changed = false;
                    for row in &mut table.rows {
                        let mut row_changed = false;
                        for cell in &mut row.cells {
                            let cell_changed = walk(&mut cell.blocks, old_formats, new_formats);
                            if cell_changed && cell.kind == docx_diff::ChangeKind::Unchanged {
                                cell.kind = docx_diff::ChangeKind::Changed;
                            }
                            row_changed |= cell_changed;
                        }
                        if row_changed && row.kind == docx_diff::ChangeKind::Unchanged {
                            row.kind = docx_diff::ChangeKind::Changed;
                        }
                        table_changed |= row_changed;
                    }
                    if table_changed && table.kind == docx_diff::ChangeKind::Unchanged {
                        table.kind = docx_diff::ChangeKind::Changed;
                    }
                    changed |= table_changed;
                }
            }
        }
        changed
    }

    walk(&mut diff.blocks, &old_formats, &new_formats);
}

fn docx_paragraph_format_signatures(path: &Path) -> Result<Vec<String>> {
    const MAX_DOCUMENT_XML_BYTES: u64 = 256 * 1024 * 1024;

    let mut archive = ZipArchive::new(File::open(path)?)?;
    let bytes = {
        let mut document = archive
            .by_name("word/document.xml")
            .context("docx has no word/document.xml")?;
        anyhow::ensure!(
            document.size() <= MAX_DOCUMENT_XML_BYTES,
            "word/document.xml exceeds formatting comparison limit"
        );
        let mut bytes = Vec::with_capacity(document.size() as usize);
        document.read_to_end(&mut bytes)?;
        bytes
    };
    let theme = match archive.by_name("word/theme/theme1.xml") {
        Ok(mut entry) => {
            anyhow::ensure!(
                entry.size() <= MAX_DOCUMENT_XML_BYTES,
                "word/theme/theme1.xml exceeds formatting comparison limit"
            );
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut bytes)?;
            DocxTheme::from_root(&parse_docx_xml(&bytes)?)
        }
        Err(zip::result::ZipError::FileNotFound) => DocxTheme::default(),
        Err(error) => return Err(error.into()),
    };
    let styles = match archive.by_name("word/styles.xml") {
        Ok(mut entry) => {
            anyhow::ensure!(
                entry.size() <= MAX_DOCUMENT_XML_BYTES,
                "word/styles.xml exceeds formatting comparison limit"
            );
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut bytes)?;
            DocxStyleSheet::from_root(&parse_docx_xml(&bytes)?, &theme)
        }
        Err(zip::result::ZipError::FileNotFound) => DocxStyleSheet::default(),
        Err(error) => return Err(error.into()),
    };
    let root = parse_docx_xml(&bytes)?;
    let mut signatures = Vec::new();
    root.visit(&mut |node| {
        if node.name == "p" {
            signatures.push(paragraph_format_signature(node, &styles, &theme));
        }
    });
    Ok(signatures)
}

#[derive(Debug, Clone, Default)]
struct DocxTheme {
    colors: BTreeMap<String, String>,
}

impl DocxTheme {
    fn from_root(root: &DocxNode) -> Self {
        let Some(color_scheme) = find_docx_descendant(root, "clrScheme") else {
            return Self::default();
        };
        let colors = color_scheme
            .children
            .iter()
            .filter_map(|entry| {
                let color = entry.children.first()?;
                Some((
                    entry.name.to_ascii_lowercase(),
                    docx_theme_color_signature(color),
                ))
            })
            .collect();
        Self { colors }
    }

    fn resolve(&self, name: &str) -> Option<&str> {
        let normalized = name.to_ascii_lowercase();
        let key = match normalized.as_str() {
            "dark1" | "text1" => "dk1",
            "light1" | "background1" => "lt1",
            "dark2" | "text2" => "dk2",
            "light2" | "background2" => "lt2",
            "hyperlink" => "hlink",
            "followedhyperlink" => "folhlink",
            other => other,
        };
        self.colors.get(key).map(String::as_str)
    }
}

fn find_docx_descendant<'a>(node: &'a DocxNode, name: &str) -> Option<&'a DocxNode> {
    if node.name == name {
        return Some(node);
    }
    node.children
        .iter()
        .find_map(|child| find_docx_descendant(child, name))
}

fn docx_theme_color_signature(color: &DocxNode) -> String {
    match color.name.as_str() {
        "srgbClr" => color
            .attr("val")
            .map(str::to_ascii_uppercase)
            .unwrap_or_else(|| canonical_docx_node(color, None)),
        "sysClr" => color
            .attr("lastClr")
            .map(str::to_ascii_uppercase)
            .unwrap_or_else(|| canonical_docx_node(color, None)),
        _ => canonical_docx_node(color, None),
    }
}

fn parse_docx_xml(bytes: &[u8]) -> Result<DocxNode> {
    let xml = String::from_utf8_lossy(bytes);
    let mut reader = XmlReader::from_str(&xml);
    reader.config_mut().trim_text(false);
    let mut stack = vec![DocxNode::default()];

    loop {
        match reader.read_event().context("failed to parse docx XML")? {
            XmlEvent::Start(start) => stack.push(docx_node(start)?),
            XmlEvent::Empty(start) => {
                let node = docx_node(start)?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                }
            }
            XmlEvent::End(_) => {
                if stack.len() > 1 {
                    let node = stack.pop().expect("docx XML stack depth checked");
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(node);
                    }
                }
            }
            XmlEvent::Text(text) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&text.xml_content().unwrap_or_default());
                }
            }
            XmlEvent::CData(data) => {
                if let Some(node) = stack.last_mut() {
                    node.text
                        .push_str(&String::from_utf8_lossy(data.into_inner().as_ref()));
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }

    stack
        .remove(0)
        .children
        .into_iter()
        .next()
        .context("docx XML document has no root element")
}

fn docx_node(start: quick_xml::events::BytesStart<'_>) -> Result<DocxNode> {
    let mut node = DocxNode {
        name: xml_local_name(start.name().as_ref()),
        ..Default::default()
    };
    for attribute in start.attributes().flatten() {
        node.attrs.push((
            String::from_utf8_lossy(attribute.key.as_ref()).into_owned(),
            attribute.unescape_value()?.into_owned(),
        ));
    }
    Ok(node)
}

fn xml_local_name(raw: &[u8]) -> String {
    let name = String::from_utf8_lossy(raw);
    name.rsplit_once(':')
        .map_or_else(|| name.to_string(), |(_, local)| local.to_string())
}

#[derive(Debug, Clone, Default)]
struct DocxStyleSheet {
    defaults: BTreeMap<String, String>,
    paragraph_defaults: BTreeMap<String, String>,
    styles: BTreeMap<String, DocxStyle>,
    default_paragraph_style: Option<String>,
    default_character_style: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct DocxStyle {
    based_on: Option<String>,
    run_properties: BTreeMap<String, String>,
    paragraph_properties: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy)]
enum DocxStylePropertyKind {
    Run,
    Paragraph,
}

impl DocxStyleSheet {
    fn from_root(root: &DocxNode, theme: &DocxTheme) -> Self {
        let defaults = root
            .child("docDefaults")
            .and_then(|defaults| defaults.child("rPrDefault"))
            .and_then(|defaults| defaults.child("rPr"))
            .map(|properties| run_properties(properties, theme))
            .unwrap_or_default();
        let paragraph_defaults = root
            .child("docDefaults")
            .and_then(|defaults| defaults.child("pPrDefault"))
            .and_then(|defaults| defaults.child("pPr"))
            .map(|properties| paragraph_properties(properties, theme))
            .unwrap_or_default();
        let mut sheet = Self {
            defaults,
            paragraph_defaults,
            ..Default::default()
        };
        for style in root.children.iter().filter(|node| node.name == "style") {
            let Some(style_id) = style.attr("styleId") else {
                continue;
            };
            let style_type = style.attr("type").unwrap_or("paragraph");
            if matches!(style.attr("default"), Some("1" | "true" | "on")) {
                match style_type {
                    "paragraph" => sheet.default_paragraph_style = Some(style_id.to_string()),
                    "character" => sheet.default_character_style = Some(style_id.to_string()),
                    _ => {}
                }
            }
            sheet.styles.insert(
                style_id.to_string(),
                DocxStyle {
                    based_on: style
                        .child("basedOn")
                        .and_then(|based_on| based_on.attr("val"))
                        .map(str::to_string),
                    run_properties: style
                        .child("rPr")
                        .map(|properties| run_properties(properties, theme))
                        .unwrap_or_default(),
                    paragraph_properties: style
                        .child("pPr")
                        .map(|properties| paragraph_properties(properties, theme))
                        .unwrap_or_default(),
                },
            );
        }
        sheet
    }

    fn resolved_style(
        &self,
        style_id: &str,
        kind: DocxStylePropertyKind,
    ) -> BTreeMap<String, String> {
        fn resolve(
            sheet: &DocxStyleSheet,
            style_id: &str,
            kind: DocxStylePropertyKind,
            visiting: &mut BTreeSet<String>,
            out: &mut BTreeMap<String, String>,
        ) {
            if !visiting.insert(style_id.to_string()) {
                return;
            }
            let Some(style) = sheet.styles.get(style_id) else {
                return;
            };
            if let Some(base) = &style.based_on {
                resolve(sheet, base, kind, visiting, out);
            }
            match kind {
                DocxStylePropertyKind::Run => out.extend(style.run_properties.clone()),
                DocxStylePropertyKind::Paragraph => out.extend(style.paragraph_properties.clone()),
            }
        }

        let mut properties = BTreeMap::new();
        resolve(self, style_id, kind, &mut BTreeSet::new(), &mut properties);
        properties
    }

    fn resolved_run_style(&self, style_id: &str) -> BTreeMap<String, String> {
        self.resolved_style(style_id, DocxStylePropertyKind::Run)
    }

    fn resolved_paragraph_style(&self, style_id: &str) -> BTreeMap<String, String> {
        self.resolved_style(style_id, DocxStylePropertyKind::Paragraph)
    }
}

fn paragraph_format_signature(
    paragraph: &DocxNode,
    styles: &DocxStyleSheet,
    theme: &DocxTheme,
) -> String {
    let mut inherited_run_properties = styles.defaults.clone();
    let mut effective_paragraph_properties = styles.paragraph_defaults.clone();
    if let Some(default_style) = &styles.default_paragraph_style {
        inherited_run_properties.extend(styles.resolved_run_style(default_style));
        effective_paragraph_properties.extend(styles.resolved_paragraph_style(default_style));
    }
    if let Some(properties) = paragraph.child("pPr") {
        if let Some(style_id) = properties
            .child("pStyle")
            .and_then(|style| style.attr("val"))
        {
            inherited_run_properties.extend(styles.resolved_run_style(style_id));
            effective_paragraph_properties.extend(styles.resolved_paragraph_style(style_id));
        }
        effective_paragraph_properties.extend(paragraph_properties(properties, theme));
        if let Some(direct_properties) = properties.child("rPr") {
            inherited_run_properties.extend(run_properties(direct_properties, theme));
        }
    }
    effective_paragraph_properties
        .retain(|name, value| !is_default_paragraph_property(name, value));
    let mut spans: Vec<(String, String)> = Vec::new();
    collect_formatted_runs(
        paragraph,
        &inherited_run_properties,
        styles,
        theme,
        &mut spans,
    );

    let mut signature = String::new();
    for (name, value) in effective_paragraph_properties {
        signature.push_str("p|");
        signature.push_str(&name);
        signature.push('=');
        signature.push_str(&value);
        signature.push(';');
    }
    for (text, format) in spans {
        signature.push_str(&text.len().to_string());
        signature.push(':');
        signature.push_str(&text);
        signature.push('|');
        signature.push_str(&format.len().to_string());
        signature.push(':');
        signature.push_str(&format);
        signature.push(';');
    }
    signature
}

fn collect_formatted_runs(
    node: &DocxNode,
    paragraph_properties: &BTreeMap<String, String>,
    styles: &DocxStyleSheet,
    theme: &DocxTheme,
    spans: &mut Vec<(String, String)>,
) {
    for child in &node.children {
        if matches!(child.name.as_str(), "del" | "moveFrom") {
            continue;
        }
        if child.name == "p" {
            continue;
        }
        if child.name == "r" {
            let text = comparison_text(&run_text(child));
            if text.is_empty() {
                continue;
            }
            let mut properties = paragraph_properties.clone();
            if let Some(default_style) = &styles.default_character_style {
                properties.extend(styles.resolved_run_style(default_style));
            }
            if let Some(direct_properties) = child.child("rPr") {
                if let Some(style_id) = direct_properties
                    .child("rStyle")
                    .and_then(|style| style.attr("val"))
                {
                    properties.extend(styles.resolved_run_style(style_id));
                }
                properties.extend(run_properties(direct_properties, theme));
            }
            properties.retain(|_, value| !is_default_run_property(value));
            let format = properties
                .into_iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join("|");
            match spans.last_mut() {
                Some((previous_text, previous_format)) if *previous_format == format => {
                    previous_text.push_str(&text);
                }
                _ => spans.push((text, format)),
            }
        } else {
            collect_formatted_runs(child, paragraph_properties, styles, theme, spans);
        }
    }
}

fn run_text(run: &DocxNode) -> String {
    fn collect(node: &DocxNode, out: &mut String) {
        match node.name.as_str() {
            "t" | "delText" => out.push_str(&node.text),
            "tab" => out.push('\t'),
            "br" | "cr" => out.push('\n'),
            "noBreakHyphen" => out.push('\u{2011}'),
            "softHyphen" => out.push('\u{ad}'),
            "instrText" => {}
            _ => {
                for child in &node.children {
                    collect(child, out);
                }
            }
        }
    }
    let mut text = String::new();
    collect(run, &mut text);
    text
}

fn run_properties(properties: &DocxNode, theme: &DocxTheme) -> BTreeMap<String, String> {
    const BOOLEAN_PROPERTIES: &[&str] = &[
        "b",
        "bCs",
        "i",
        "iCs",
        "caps",
        "smallCaps",
        "strike",
        "dstrike",
        "outline",
        "shadow",
        "emboss",
        "imprint",
        "vanish",
        "rtl",
        "cs",
        "specVanish",
    ];
    const NON_VISUAL_PROPERTIES: &[&str] =
        &["rStyle", "rPrChange", "ins", "del", "moveFrom", "moveTo"];

    properties
        .children
        .iter()
        .filter(|property| !NON_VISUAL_PROPERTIES.contains(&property.name.as_str()))
        .map(|property| {
            let value = if BOOLEAN_PROPERTIES.contains(&property.name.as_str()) {
                if matches!(property.attr("val"), Some("0" | "false" | "off")) {
                    "0".to_string()
                } else {
                    "1".to_string()
                }
            } else {
                canonical_docx_node(property, Some(theme))
            };
            (property.name.clone(), value)
        })
        .collect()
}

fn paragraph_properties(properties: &DocxNode, theme: &DocxTheme) -> BTreeMap<String, String> {
    const BOOLEAN_PROPERTIES: &[&str] = &[
        "keepNext",
        "keepLines",
        "pageBreakBefore",
        "widowControl",
        "suppressLineNumbers",
        "suppressAutoHyphens",
        "kinsoku",
        "wordWrap",
        "overflowPunct",
        "topLinePunct",
        "autoSpaceDE",
        "autoSpaceDN",
        "bidi",
        "adjustRightInd",
        "snapToGrid",
        "contextualSpacing",
        "mirrorIndents",
    ];
    const NON_VISUAL_PROPERTIES: &[&str] = &["pStyle", "rPr", "pPrChange", "sectPr"];

    properties
        .children
        .iter()
        .filter(|property| !NON_VISUAL_PROPERTIES.contains(&property.name.as_str()))
        .map(|property| {
            let value = if BOOLEAN_PROPERTIES.contains(&property.name.as_str()) {
                if matches!(property.attr("val"), Some("0" | "false" | "off")) {
                    "0".to_string()
                } else {
                    "1".to_string()
                }
            } else {
                canonical_docx_node(property, Some(theme))
            };
            (property.name.clone(), value)
        })
        .collect()
}

fn is_default_paragraph_property(name: &str, value: &str) -> bool {
    fn only_zero_attributes(value: &str) -> bool {
        let mut attributes = value.split('|').skip(1);
        let has_attributes = attributes.clone().next().is_some();
        has_attributes
            && attributes.all(|attribute| {
                attribute
                    .split_once('=')
                    .is_some_and(|(_, value)| value.parse::<i64>() == Ok(0))
            })
    }

    value == "0"
        || matches!((name, value), ("widowControl", "1"))
        || matches!(value, "jc|val=left" | "jc|val=start")
        || matches!(value, "outlineLvl|val=9")
        || (matches!(name, "ind" | "spacing") && (value == name || only_zero_attributes(value)))
        || (name == "tabs" && value == "tabs")
}

fn is_default_run_property(value: &str) -> bool {
    matches!(
        value,
        "0" | "color|val=AUTO" | "u|val=none" | "vertAlign|val=baseline"
    )
}

#[derive(Clone, Copy)]
enum DocxThemeModifier {
    Tint(u8),
    Shade(u8),
}

fn docx_theme_modifier(
    node: &DocxNode,
    tint_attribute: &str,
    shade_attribute: &str,
) -> Option<DocxThemeModifier> {
    let parse = |value: &str| {
        (value.len() == 2)
            .then(|| u8::from_str_radix(value, 16).ok())
            .flatten()
    };
    node.attr(tint_attribute)
        .and_then(parse)
        .map(DocxThemeModifier::Tint)
        .or_else(|| {
            node.attr(shade_attribute)
                .and_then(parse)
                .map(DocxThemeModifier::Shade)
        })
}

fn apply_docx_theme_modifier(color: &str, modifier: DocxThemeModifier) -> Option<String> {
    if color.len() != 6 || !color.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let red = i32::from(u8::from_str_radix(&color[0..2], 16).ok()?);
    let green = i32::from(u8::from_str_radix(&color[2..4], 16).ok()?);
    let blue = i32::from(u8::from_str_radix(&color[4..6], 16).ok()?);
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    let mut luminance = (maximum + minimum) / 2;
    let (hue, saturation) = if maximum == minimum {
        (170, 0)
    } else {
        let delta = maximum - minimum;
        let saturation = if luminance <= 127 {
            (delta * 255 + (maximum + minimum) / 2) / (maximum + minimum)
        } else {
            (delta * 255 + (510 - maximum - minimum) / 2) / (510 - maximum - minimum)
        };
        let red_delta = ((maximum - red) * 42 + delta / 2) / delta;
        let green_delta = ((maximum - green) * 42 + delta / 2) / delta;
        let blue_delta = ((maximum - blue) * 42 + delta / 2) / delta;
        let mut hue = if red == maximum {
            blue_delta - green_delta
        } else if green == maximum {
            85 + red_delta - blue_delta
        } else {
            170 + green_delta - red_delta
        };
        if hue < 0 {
            hue += 255;
        } else if hue > 255 {
            hue -= 255;
        }
        (hue, saturation)
    };
    luminance = match modifier {
        DocxThemeModifier::Tint(value) => {
            luminance * i32::from(value) / 255 + 255 - i32::from(value)
        }
        DocxThemeModifier::Shade(value) => luminance * i32::from(value) / 255,
    };

    fn hue_to_rgb(low: i32, high: i32, mut hue: i32) -> i32 {
        if hue < 0 {
            hue += 255;
        }
        if hue > 255 {
            hue -= 255;
        }
        if hue < 42 {
            low + ((high - low) * hue + 21) / 42
        } else if hue < 127 {
            high
        } else if hue < 170 {
            low + ((high - low) * (170 - hue) + 21) / 42
        } else {
            low
        }
    }

    let (red, green, blue) = if saturation == 0 {
        (luminance, luminance, luminance)
    } else {
        let high = if luminance <= 127 {
            (luminance * (255 + saturation) + 127) / 255
        } else {
            luminance + saturation - (luminance * saturation + 127) / 255
        };
        let low = 2 * luminance - high;
        (
            (hue_to_rgb(low, high, hue + 85) * 255 + 127) / 255,
            (hue_to_rgb(low, high, hue) * 255 + 127) / 255,
            (hue_to_rgb(low, high, hue - 85) * 255 + 127) / 255,
        )
    };
    Some(format!("{red:02X}{green:02X}{blue:02X}"))
}

fn canonical_docx_node(node: &DocxNode, theme: Option<&DocxTheme>) -> String {
    let theme_color_attribute = if node.name == "shd" { "color" } else { "val" };
    let theme_color_modifier = docx_theme_modifier(node, "themeTint", "themeShade");
    let theme_fill_modifier = docx_theme_modifier(node, "themeFillTint", "themeFillShade");
    let resolved_theme_color = theme
        .filter(|_| matches!(node.name.as_str(), "color" | "shd"))
        .and_then(|theme| {
            node.attr("themeColor")
                .and_then(|color| theme.resolve(color))
        })
        .map(|color| {
            theme_color_modifier
                .and_then(|modifier| apply_docx_theme_modifier(color, modifier))
                .unwrap_or_else(|| color.to_string())
        });
    let resolved_theme_fill = theme
        .filter(|_| node.name == "shd")
        .and_then(|theme| {
            node.attr("themeFill")
                .and_then(|color| theme.resolve(color))
        })
        .map(|color| {
            theme_fill_modifier
                .and_then(|modifier| apply_docx_theme_modifier(color, modifier))
                .unwrap_or_else(|| color.to_string())
        });
    let mut attributes: Vec<(String, String)> = node
        .attrs
        .iter()
        .filter(|(name, _)| !name.starts_with("xmlns") && !name.contains("rsid"))
        .filter_map(|(name, value)| {
            let name = name.rsplit(':').next().unwrap_or(name).to_string();
            if (name == "themeColor" && resolved_theme_color.is_some())
                || (name == "themeFill" && resolved_theme_fill.is_some())
                || (name == theme_color_attribute && resolved_theme_color.is_some())
                || (name == "fill" && resolved_theme_fill.is_some())
                || (matches!(name.as_str(), "themeTint" | "themeShade")
                    && resolved_theme_color.is_some()
                    && theme_color_modifier.is_some())
                || (matches!(name.as_str(), "themeFillTint" | "themeFillShade")
                    && resolved_theme_fill.is_some()
                    && theme_fill_modifier.is_some())
            {
                return None;
            }
            let value = if name == "color"
                || matches!(node.name.as_str(), "color" | "shd")
                    && matches!(
                        name.as_str(),
                        "val" | "color" | "fill" | "themeColor" | "themeFill"
                    ) {
                value.to_ascii_uppercase()
            } else if matches!(
                node.name.as_str(),
                "ind" | "spacing" | "sz" | "szCs" | "kern" | "position" | "w"
            ) {
                value
                    .parse::<i64>()
                    .map_or_else(|_| value.clone(), |value| value.to_string())
            } else {
                value.clone()
            };
            Some((name, value))
        })
        .collect();
    if let Some(color) = resolved_theme_color {
        attributes.push((theme_color_attribute.to_string(), color));
    }
    if let Some(color) = resolved_theme_fill {
        attributes.push(("fill".to_string(), color));
    }
    attributes.sort();
    let mut signature = node.name.clone();
    for (name, value) in attributes {
        signature.push('|');
        signature.push_str(&name);
        signature.push('=');
        signature.push_str(&value);
    }
    for child in &node.children {
        signature.push('{');
        signature.push_str(&canonical_docx_node(child, theme));
        signature.push('}');
    }
    signature
}

fn summarize_docx(blocks: &[docx_diff::BlockChange]) -> docx_diff::DiffSummary {
    fn changed_outside_nested_tables(row: &docx_diff::RowChange) -> bool {
        row.cells.iter().any(|cell| {
            cell.kind != docx_diff::ChangeKind::Unchanged
                && cell.blocks.iter().any(|block| match block {
                    docx_diff::BlockChange::Paragraph(change) => {
                        change.kind != docx_diff::ChangeKind::Unchanged
                    }
                    docx_diff::BlockChange::Table(change) => matches!(
                        change.kind,
                        docx_diff::ChangeKind::Added | docx_diff::ChangeKind::Removed
                    ),
                })
        })
    }

    fn accumulate(summary: &mut docx_diff::DiffSummary, block: &docx_diff::BlockChange) {
        match block {
            docx_diff::BlockChange::Paragraph(change) => match change.kind {
                docx_diff::ChangeKind::Added => summary.paragraphs_added += 1,
                docx_diff::ChangeKind::Removed => summary.paragraphs_removed += 1,
                docx_diff::ChangeKind::Changed => summary.paragraphs_changed += 1,
                docx_diff::ChangeKind::Moved => summary.paragraphs_moved += 1,
                docx_diff::ChangeKind::Unchanged => {}
            },
            docx_diff::BlockChange::Table(table) => {
                match table.kind {
                    docx_diff::ChangeKind::Added => {
                        summary.tables_added += 1;
                        return;
                    }
                    docx_diff::ChangeKind::Removed => {
                        summary.tables_removed += 1;
                        return;
                    }
                    _ => {}
                }
                for row in &table.rows {
                    match row.kind {
                        docx_diff::ChangeKind::Added => summary.table_rows_added += 1,
                        docx_diff::ChangeKind::Removed => summary.table_rows_removed += 1,
                        docx_diff::ChangeKind::Changed if changed_outside_nested_tables(row) => {
                            summary.table_rows_changed += 1;
                        }
                        _ => {}
                    }
                    if matches!(
                        row.kind,
                        docx_diff::ChangeKind::Added | docx_diff::ChangeKind::Removed
                    ) {
                        continue;
                    }
                    for cell in &row.cells {
                        for inner in &cell.blocks {
                            accumulate(summary, inner);
                        }
                    }
                }
            }
        }
    }

    let mut summary = docx_diff::DiffSummary::default();
    for block in blocks {
        accumulate(&mut summary, block);
    }
    summary
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use std::collections::BTreeSet;
    use std::io::Write;

    use proptest::prelude::*;
    use proptest::test_runner::TestCaseResult;
    use unicode_segmentation::UnicodeSegmentation;

    use super::*;

    fn unicode_text() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                3 => any::<char>().prop_map(|character| character.to_string()),
                2 => prop::sample::select(vec![
                    "a", "b", "日", "本", "\u{0301}", "\u{200d}", "👩", "👨", "💻", "👍",
                    "🏽", "🇯", "🇵", "\r", "\n", " ", "　",
                ])
                .prop_map(str::to_owned),
            ],
            0..24,
        )
        .prop_map(|parts| parts.concat())
    }

    fn grapheme_boundaries(text: &str) -> BTreeSet<usize> {
        std::iter::once(0)
            .chain(
                text.grapheme_indices(true)
                    .map(|(start, grapheme)| start + grapheme.len()),
            )
            .collect()
    }

    fn write_docx(path: &std::path::Path, text: &str) {
        write_docx_body(path, &format!("<w:r><w:t>{text}</w:t></w:r>"));
    }

    fn write_docx_body(path: &std::path::Path, paragraph_body: &str) {
        write_docx_body_with_styles(path, paragraph_body, None);
    }

    fn write_docx_body_with_styles(
        path: &std::path::Path,
        paragraph_body: &str,
        styles: Option<&str>,
    ) {
        write_docx_body_with_parts(path, paragraph_body, styles, None);
    }

    fn write_docx_body_with_theme(path: &std::path::Path, paragraph_body: &str, theme: &str) {
        write_docx_body_with_parts(path, paragraph_body, None, Some(theme));
    }

    fn write_docx_body_with_parts(
        path: &std::path::Path,
        paragraph_body: &str,
        styles: Option<&str>,
        theme: Option<&str>,
    ) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        let mut content_types = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_string();
        if styles.is_some() {
            content_types = content_types.replace(
                "</Types>",
                r#"<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>"#,
            );
        }
        if theme.is_some() {
            content_types = content_types.replace(
                "</Types>",
                r#"<Override PartName="/word/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/></Types>"#,
            );
        }
        let mut parts = vec![
            ("[Content_Types].xml".to_string(), content_types),
            (
                "_rels/.rels".to_string(),
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_string(),
            ),
            (
                "word/document.xml".to_string(),
                format!(r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"><w:body><w:p w14:paraId="00000001">{paragraph_body}</w:p></w:body></w:document>"#),
            ),
        ];
        if styles.is_some() || theme.is_some() {
            let mut relationships =
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"></Relationships>"#.to_string();
            if styles.is_some() {
                relationships = relationships.replace(
                    "</Relationships>",
                    r#"<Relationship Id="rIdStyles" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
                );
            }
            if theme.is_some() {
                relationships = relationships.replace(
                    "</Relationships>",
                    r#"<Relationship Id="rIdTheme" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="theme/theme1.xml"/></Relationships>"#,
                );
            }
            parts.push(("word/_rels/document.xml.rels".to_string(), relationships));
        }
        if let Some(styles) = styles {
            parts.push(("word/styles.xml".to_string(), styles.to_string()));
        }
        if let Some(theme) = theme {
            parts.push(("word/theme/theme1.xml".to_string(), theme.to_string()));
        }
        for (name, body) in parts {
            zip.start_file(name, options).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    fn assert_valid_segments(old: &str, new: &str, segments: &[Segment]) -> TestCaseResult {
        let old_boundaries = grapheme_boundaries(old);
        let new_boundaries = grapheme_boundaries(new);
        let mut old_at = 0;
        let mut new_at = 0;

        for segment in segments {
            prop_assert!(!segment.text.is_empty(), "空の差分区間がある");
            match segment.kind {
                SegmentKind::Equal => {
                    prop_assert!(old[old_at..].starts_with(&segment.text));
                    prop_assert!(new[new_at..].starts_with(&segment.text));
                    old_at += segment.text.len();
                    new_at += segment.text.len();
                    prop_assert!(old_boundaries.contains(&old_at));
                    prop_assert!(new_boundaries.contains(&new_at));
                }
                SegmentKind::Delete => {
                    prop_assert!(old[old_at..].starts_with(&segment.text));
                    old_at += segment.text.len();
                    prop_assert!(old_boundaries.contains(&old_at));
                }
                SegmentKind::Insert => {
                    prop_assert!(new[new_at..].starts_with(&segment.text));
                    new_at += segment.text.len();
                    prop_assert!(new_boundaries.contains(&new_at));
                }
            }
        }

        prop_assert_eq!(old_at, old.len(), "変更前を完全に復元できない");
        prop_assert_eq!(new_at, new.len(), "変更後を完全に復元できない");
        for pair in segments.windows(2) {
            prop_assert_ne!(pair[0].kind, pair[1].kind, "同種の区間が連続している");
        }
        Ok(())
    }

    #[test]
    fn 辞書が無ければ_書記素クラスタ単位に戻る() {
        let segmenter = Segmenter::char_only();
        assert!(!segmenter.has_dictionary());
        let segments = segmenter.segments("あい", "あう");
        let joined: String = segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(joined, "あいう");
    }

    #[test]
    fn 内訳を繋ぐと元の文字列に戻る() {
        let segmenter = Segmenter::char_only();
        let old = "第3条 委託料は月額 500,000 円とする。";
        let new = "第3条 委託料は月額 550,000 円とする。";
        let segments = segmenter.segments(old, new);

        let restored_old: String = segments
            .iter()
            .filter(|s| s.kind != SegmentKind::Insert)
            .map(|s| s.text.as_str())
            .collect();
        let restored_new: String = segments
            .iter()
            .filter(|s| s.kind != SegmentKind::Delete)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(restored_old, old, "変更前が戻らない");
        assert_eq!(restored_new, new, "変更後が戻らない");
    }

    #[test]
    fn 同じ文字列なら内訳は_equal_だけ() {
        let segmenter = Segmenter::char_only();
        let segments = segmenter.segments("同じです", "同じです");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].kind, SegmentKind::Equal);
    }

    #[test]
    fn DOCXのUnicode正規化形式だけの違いは変更にしない() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        write_docx(&old, "Caf\u{e9}");
        write_docx(&new, "Cafe\u{301}");
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(!diff.has_changes(), "{diff:?}");
        assert_eq!(diff.summary.paragraphs_changed, 0);
    }

    #[test]
    fn DOCXのソフトハイフンだけの違いは変更にしない() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        write_docx(&old, "international");
        write_docx(&new, "inter\u{ad}national");
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(!diff.has_changes(), "{diff:?}");
        assert_eq!(diff.summary.paragraphs_changed, 0);
    }

    #[test]
    fn DOCXの本文が同じでも太字変更を検出する() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        write_docx_body(&old, "<w:r><w:t>重要</w:t></w:r>");
        write_docx_body(&new, "<w:r><w:rPr><w:b/></w:rPr><w:t>重要</w:t></w:r>");
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(diff.has_changes(), "{diff:?}");
        assert_eq!(diff.summary.paragraphs_changed, 1, "{diff:?}");
    }

    #[test]
    fn DOCXの本文が同じでも段落配置変更を検出する() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        write_docx_body(&old, "<w:r><w:t>同じ本文</w:t></w:r>");
        write_docx_body(
            &new,
            "<w:pPr><w:jc w:val=\"center\"/></w:pPr><w:r><w:t>同じ本文</w:t></w:r>",
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(diff.has_changes(), "{diff:?}");
        assert_eq!(diff.summary.paragraphs_changed, 1, "{diff:?}");
    }

    #[test]
    fn DOCXの文字スタイル定義変更を検出する() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        let body = "<w:r><w:rPr><w:rStyle w:val=\"Emphasis\"/></w:rPr><w:t>重要</w:t></w:r>";
        write_docx_body_with_styles(
            &old,
            body,
            Some(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="character" w:styleId="Emphasis"><w:rPr><w:color w:val="FF0000"/></w:rPr></w:style></w:styles>"#,
            ),
        );
        write_docx_body_with_styles(
            &new,
            body,
            Some(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="character" w:styleId="Emphasis"><w:rPr><w:color w:val="0000FF"/></w:rPr></w:style></w:styles>"#,
            ),
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(diff.has_changes(), "{diff:?}");
        assert_eq!(diff.summary.paragraphs_changed, 1, "{diff:?}");
    }

    #[test]
    fn DOCXの未使用文字スタイル定義変更は無視する() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        let body = "<w:r><w:t>通常</w:t></w:r>";
        write_docx_body_with_styles(
            &old,
            body,
            Some(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="character" w:styleId="Unused"><w:rPr><w:color w:val="FF0000"/></w:rPr></w:style></w:styles>"#,
            ),
        );
        write_docx_body_with_styles(
            &new,
            body,
            Some(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="character" w:styleId="Unused"><w:rPr><w:color w:val="0000FF"/></w:rPr></w:style></w:styles>"#,
            ),
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(!diff.has_changes(), "{diff:?}");
    }

    #[test]
    fn DOCXの見た目が同じ文字スタイルID変更は無視する() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        write_docx_body_with_styles(
            &old,
            "<w:r><w:rPr><w:rStyle w:val=\"OldStyle\"/></w:rPr><w:t>重要</w:t></w:r>",
            Some(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="character" w:styleId="OldStyle"><w:rPr><w:b/></w:rPr></w:style></w:styles>"#,
            ),
        );
        write_docx_body_with_styles(
            &new,
            "<w:r><w:rPr><w:rStyle w:val=\"NewStyle\"/></w:rPr><w:t>重要</w:t></w:r>",
            Some(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="character" w:styleId="NewStyle"><w:rPr><w:b/></w:rPr></w:style></w:styles>"#,
            ),
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(!diff.has_changes(), "{diff:?}");
    }

    #[test]
    fn DOCXの継承された段落スタイル変更を検出する() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        let body = "<w:pPr><w:pStyle w:val=\"Heading\"/></w:pPr><w:r><w:t>見出し</w:t></w:r>";
        write_docx_body_with_styles(
            &old,
            body,
            Some(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Base"><w:rPr><w:color w:val="FF0000"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading"><w:basedOn w:val="Base"/></w:style></w:styles>"#,
            ),
        );
        write_docx_body_with_styles(
            &new,
            body,
            Some(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Base"><w:rPr><w:color w:val="0000FF"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading"><w:basedOn w:val="Base"/></w:style></w:styles>"#,
            ),
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(diff.has_changes(), "{diff:?}");
        assert_eq!(diff.summary.paragraphs_changed, 1, "{diff:?}");
    }

    #[test]
    fn DOCXの継承された段落レイアウト変更を検出する() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        let body = "<w:pPr><w:pStyle w:val=\"Heading\"/></w:pPr><w:r><w:t>見出し</w:t></w:r>";
        write_docx_body_with_styles(
            &old,
            body,
            Some(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Base"><w:pPr><w:ind w:left="720"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="Heading"><w:basedOn w:val="Base"/></w:style></w:styles>"#,
            ),
        );
        write_docx_body_with_styles(
            &new,
            body,
            Some(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Base"><w:pPr><w:ind w:left="1440"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="Heading"><w:basedOn w:val="Base"/></w:style></w:styles>"#,
            ),
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(diff.has_changes(), "{diff:?}");
        assert_eq!(diff.summary.paragraphs_changed, 1, "{diff:?}");
    }

    #[test]
    fn DOCXの参照テーマ色変更を検出する() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        let body = "<w:r><w:rPr><w:color w:val=\"000000\" w:themeColor=\"accent1\"/></w:rPr><w:t>重要</w:t></w:r>";
        write_docx_body_with_theme(
            &old,
            body,
            r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="Office"><a:accent1><a:srgbClr val="FF0000"/></a:accent1><a:accent2><a:srgbClr val="00FF00"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#,
        );
        write_docx_body_with_theme(
            &new,
            body,
            r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="Office"><a:accent1><a:srgbClr val="0000FF"/></a:accent1><a:accent2><a:srgbClr val="00FF00"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#,
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(diff.has_changes(), "{diff:?}");
        assert_eq!(diff.summary.paragraphs_changed, 1, "{diff:?}");
    }

    #[test]
    fn DOCXの未参照テーマ色変更は無視する() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        let body = "<w:r><w:rPr><w:color w:val=\"000000\" w:themeColor=\"accent1\"/></w:rPr><w:t>重要</w:t></w:r>";
        write_docx_body_with_theme(
            &old,
            body,
            r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="Office"><a:accent1><a:srgbClr val="FF0000"/></a:accent1><a:accent2><a:srgbClr val="00FF00"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#,
        );
        write_docx_body_with_theme(
            &new,
            body,
            r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="Office"><a:accent1><a:srgbClr val="FF0000"/></a:accent1><a:accent2><a:srgbClr val="0000FF"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#,
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(!diff.has_changes(), "{diff:?}");
    }

    #[test]
    fn DOCXのテーマ色と同じ直接色指定は変更にしない() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        let theme = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="Office"><a:accent1><a:srgbClr val="FF0000"/></a:accent1></a:clrScheme></a:themeElements></a:theme>"#;
        write_docx_body_with_theme(
            &old,
            "<w:r><w:rPr><w:color w:val=\"000000\" w:themeColor=\"accent1\"/></w:rPr><w:t>重要</w:t></w:r>",
            theme,
        );
        write_docx_body_with_theme(
            &new,
            "<w:r><w:rPr><w:color w:val=\"FF0000\"/></w:rPr><w:t>重要</w:t></w:r>",
            theme,
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(!diff.has_changes(), "{diff:?}");
    }

    #[test]
    fn DOCXのテーマ網掛けと同じ直接色指定は変更にしない() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        let theme = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:themeElements><a:clrScheme name="Office"><a:accent1><a:srgbClr val="FF0000"/></a:accent1><a:accent2><a:srgbClr val="00FF00"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#;
        write_docx_body_with_theme(
            &old,
            "<w:r><w:rPr><w:shd w:val=\"clear\" w:color=\"000000\" w:fill=\"000000\" w:themeColor=\"accent1\" w:themeFill=\"accent2\"/></w:rPr><w:t>重要</w:t></w:r>",
            theme,
        );
        write_docx_body_with_theme(
            &new,
            "<w:r><w:rPr><w:shd w:val=\"clear\" w:color=\"FF0000\" w:fill=\"00FF00\"/></w:rPr><w:t>重要</w:t></w:r>",
            theme,
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(!diff.has_changes(), "{diff:?}");
    }

    #[test]
    fn DOCXの明示的な既定書式は変更にしない() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        write_docx_body(&old, "<w:r><w:t>通常</w:t></w:r>");
        write_docx_body(
            &new,
            "<w:r><w:rPr><w:b w:val=\"false\"/><w:i w:val=\"0\"/><w:color w:val=\"auto\"/><w:u w:val=\"none\"/><w:vertAlign w:val=\"baseline\"/></w:rPr><w:t>通常</w:t></w:r>",
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(!diff.has_changes(), "{diff:?}");
    }

    #[test]
    fn DOCXの明示的な既定段落書式は変更にしない() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        write_docx_body(&old, "<w:r><w:t>通常</w:t></w:r>");
        write_docx_body(
            &new,
            "<w:pPr><w:jc w:val=\"left\"/><w:ind w:left=\"0\" w:right=\"0\"/><w:spacing w:before=\"0\" w:after=\"0\"/><w:widowControl/></w:pPr><w:r><w:t>通常</w:t></w:r>",
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(!diff.has_changes(), "{diff:?}");
    }

    #[test]
    fn DOCXの直接既定段落書式は継承値を上書きする() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        write_docx_body_with_styles(
            &old,
            "<w:pPr><w:pStyle w:val=\"Base\"/><w:jc w:val=\"left\"/></w:pPr><w:r><w:t>通常</w:t></w:r>",
            Some(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Base"><w:pPr><w:jc w:val="center"/></w:pPr></w:style></w:styles>"#,
            ),
        );
        write_docx_body_with_styles(
            &new,
            "<w:pPr><w:pStyle w:val=\"Base\"/></w:pPr><w:r><w:t>通常</w:t></w:r>",
            Some(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:styleId="Base"><w:pPr><w:jc w:val="left"/></w:pPr></w:style></w:styles>"#,
            ),
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(!diff.has_changes(), "{diff:?}");
    }

    #[test]
    fn DOCXの同一書式run分割は変更にしない() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        write_docx_body(&old, "<w:r><w:rPr><w:b/></w:rPr><w:t>重要事項</w:t></w:r>");
        write_docx_body(
            &new,
            "<w:r><w:rPr><w:b/></w:rPr><w:t>重要</w:t></w:r><w:r><w:rPr><w:b/></w:rPr><w:t>事項</w:t></w:r>",
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(!diff.has_changes(), "{diff:?}");
    }

    #[test]
    fn DOCXの部分的な文字書式変更を検出する() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.docx");
        let new = dir.path().join("new.docx");
        write_docx_body(&old, "<w:r><w:t>重要事項</w:t></w:r>");
        write_docx_body(
            &new,
            "<w:r><w:rPr><w:color w:val=\"FF0000\"/></w:rPr><w:t>重要</w:t></w:r><w:r><w:t>事項</w:t></w:r>",
        );
        let mut diff = docx_diff::diff_files(
            &old,
            &new,
            &docx_diff::Limits::default(),
            &docx_diff::DiffOptions::default(),
        )
        .unwrap();

        resegment_docx(&mut diff, &Segmenter::char_only());

        assert!(diff.has_changes(), "{diff:?}");
        assert_eq!(diff.summary.paragraphs_changed, 1, "{diff:?}");
    }

    #[test]
    fn 絵文字の書記素クラスタを途中で分割しない() {
        let segments = Segmenter::char_only().segments("👩🏽‍💻", "👨🏽‍💻");
        assert_valid_segments("👩🏽‍💻", "👨🏽‍💻", &segments).unwrap();
    }

    #[test]
    fn 形態素の境界が書記素の途中なら結合する() {
        let units = vec!["👩", "🏽", "\u{200d}", "💻"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(merge_at_grapheme_boundaries("👩🏽‍💻", units), vec!["👩🏽‍💻"]);
    }

    #[test]
    fn DOCXのOfficeテーマ明暗補正例を直接色へ解決する() {
        let theme = DocxTheme::from_root(
            &parse_docx_xml(
                br#"<a:theme xmlns:a="a"><a:themeElements><a:clrScheme><a:accent1><a:srgbClr val="4F81BD"/></a:accent1><a:accent2><a:srgbClr val="C0504D"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#,
            )
            .unwrap(),
        );
        let tinted = parse_docx_xml(
            br#"<w:color xmlns:w="w" w:val="000000" w:themeColor="accent1" w:themeTint="99"/>"#,
        )
        .unwrap();
        let shaded = parse_docx_xml(
            br#"<w:color xmlns:w="w" w:val="000000" w:themeColor="accent2" w:themeShade="BF"/>"#,
        )
        .unwrap();

        assert_eq!(
            canonical_docx_node(&tinted, Some(&theme)),
            "color|val=95B3D7"
        );
        assert_eq!(
            canonical_docx_node(&shaded, Some(&theme)),
            "color|val=943634"
        );
    }

    #[test]
    fn DOCXのテーマ網掛け前景背景補正を直接色へ解決する() {
        let theme = DocxTheme::from_root(
            &parse_docx_xml(
                br#"<a:theme xmlns:a="a"><a:themeElements><a:clrScheme><a:accent1><a:srgbClr val="4F81BD"/></a:accent1><a:accent2><a:srgbClr val="C0504D"/></a:accent2></a:clrScheme></a:themeElements></a:theme>"#,
            )
            .unwrap(),
        );
        let themed = parse_docx_xml(
            br#"<w:shd xmlns:w="w" w:val="clear" w:color="000000" w:fill="000000" w:themeColor="accent1" w:themeTint="99" w:themeFill="accent2" w:themeFillShade="BF"/>"#,
        )
        .unwrap();
        let direct = parse_docx_xml(
            br#"<w:shd xmlns:w="w" w:val="clear" w:color="95B3D7" w:fill="943634"/>"#,
        )
        .unwrap();

        assert_eq!(
            canonical_docx_node(&themed, Some(&theme)),
            canonical_docx_node(&direct, Some(&theme)),
        );
    }

    proptest! {
        #[test]
        fn DOCXのテーマ明暗補正はレンダリング済み直接色と等価になる(
            channel in any::<u8>(),
            modifier in any::<u8>(),
            tint in any::<bool>(),
        ) {
            let rendered = if tint {
                (u16::from(channel) * u16::from(modifier) / 255 + 255 - u16::from(modifier)) as u8
            } else {
                (u16::from(channel) * u16::from(modifier) / 255) as u8
            };
            let theme_xml = format!(
                r#"<a:theme xmlns:a="a"><a:themeElements><a:clrScheme><a:accent1><a:srgbClr val="{channel:02X}{channel:02X}{channel:02X}"/></a:accent1></a:clrScheme></a:themeElements></a:theme>"#
            );
            let theme = DocxTheme::from_root(
                &parse_docx_xml(theme_xml.as_bytes()).unwrap(),
            );
            let modifier_name = if tint { "themeTint" } else { "themeShade" };
            let themed_xml = format!(
                r#"<w:color xmlns:w="w" w:val="000000" w:themeColor="accent1" w:{modifier_name}="{modifier:02X}"/>"#
            );
            let direct_xml = format!(
                r#"<w:color xmlns:w="w" w:val="{rendered:02X}{rendered:02X}{rendered:02X}"/>"#
            );
            let themed = parse_docx_xml(themed_xml.as_bytes()).unwrap();
            let direct = parse_docx_xml(direct_xml.as_bytes()).unwrap();

            prop_assert_eq!(
                canonical_docx_node(&themed, Some(&theme)),
                canonical_docx_node(&direct, Some(&theme)),
            );
        }

        #[test]
        fn DOCXの同値なフォントサイズ整数表記は同じ文字書式になる(
            value in 1i64..=3_276,
            padding in 1usize..5,
            explicit_plus in any::<bool>(),
        ) {
            let sign = if explicit_plus { "+" } else { "" };
            let padded = format!("{sign}{}{value}", "0".repeat(padding));
            let canonical_node = parse_docx_xml(
                format!(r#"<w:rPr xmlns:w="w"><w:sz w:val="{value}"/></w:rPr>"#).as_bytes(),
            )
            .unwrap();
            let padded_node = parse_docx_xml(
                format!(r#"<w:rPr xmlns:w="w"><w:sz w:val="{padded}"/></w:rPr>"#).as_bytes(),
            )
            .unwrap();

            prop_assert_eq!(
                run_properties(&canonical_node, &DocxTheme::default()),
                run_properties(&padded_node, &DocxTheme::default()),
            );
        }

        #[test]
        fn DOCXの同値な段落間隔整数表記は同じ段落書式になる(
            value in 0i64..=100_000,
            padding in 1usize..5,
            explicit_plus in any::<bool>(),
        ) {
            let sign = if explicit_plus { "+" } else { "" };
            let padded = format!("{sign}{}{value}", "0".repeat(padding));
            let canonical_node = parse_docx_xml(
                format!(r#"<w:pPr xmlns:w="w"><w:spacing w:before="{value}"/></w:pPr>"#).as_bytes(),
            )
            .unwrap();
            let padded_node = parse_docx_xml(
                format!(r#"<w:pPr xmlns:w="w"><w:spacing w:before="{padded}"/></w:pPr>"#).as_bytes(),
            )
            .unwrap();

            prop_assert_eq!(
                paragraph_properties(&canonical_node, &DocxTheme::default()),
                paragraph_properties(&padded_node, &DocxTheme::default()),
            );
        }

        #[test]
        fn DOCXの同値なインデント整数表記は同じ段落書式になる(
            value in -100_000i64..=100_000,
            padding in 1usize..5,
            explicit_plus in any::<bool>(),
        ) {
            let canonical = value.to_string();
            let sign = if value < 0 {
                "-"
            } else if explicit_plus {
                "+"
            } else {
                ""
            };
            let padded = format!("{sign}{}{value}", "0".repeat(padding), value = value.abs());
            let canonical_node = parse_docx_xml(
                format!(r#"<w:pPr xmlns:w="w"><w:ind w:left="{canonical}"/></w:pPr>"#).as_bytes(),
            )
            .unwrap();
            let padded_node = parse_docx_xml(
                format!(r#"<w:pPr xmlns:w="w"><w:ind w:left="{padded}"/></w:pPr>"#).as_bytes(),
            )
            .unwrap();

            prop_assert_eq!(
                paragraph_properties(&canonical_node, &DocxTheme::default()),
                paragraph_properties(&padded_node, &DocxTheme::default()),
            );
        }

        #[test]
        fn DOCXの下線と罫線の16進色は大文字小文字を区別しない(
            element in prop::sample::select(vec![
                "u", "bdr", "top", "bottom", "left", "right", "start", "end",
                "insideH", "insideV", "between", "bar", "tl2br", "tr2bl",
            ]),
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
        ) {
            let rgb = format!("{red:02X}{green:02X}{blue:02X}");
            let changed_rgb = format!("{:02X}{green:02X}{blue:02X}", red.wrapping_add(1));
            let value = "single";
            let uppercase = parse_docx_xml(
                format!(r#"<w:{element} xmlns:w="w" w:val="{value}" w:color="{rgb}"/>"#)
                    .as_bytes(),
            )
            .unwrap();
            let lowercase = parse_docx_xml(
                format!(
                    r#"<w:{element} xmlns:w="w" w:val="{value}" w:color="{}"/>"#,
                    rgb.to_ascii_lowercase(),
                )
                .as_bytes(),
            )
            .unwrap();
            let changed = parse_docx_xml(
                format!(
                    r#"<w:{element} xmlns:w="w" w:val="{value}" w:color="{changed_rgb}"/>"#,
                )
                .as_bytes(),
            )
            .unwrap();

            prop_assert_eq!(
                canonical_docx_node(&uppercase, None),
                canonical_docx_node(&lowercase, None),
            );
            prop_assert_ne!(
                canonical_docx_node(&uppercase, None),
                canonical_docx_node(&changed, None),
            );
        }

        #[test]
        fn DOCXの段落ゼロ値表現はすべて既定値になる(
            zero_digits in 1usize..5,
            sign in 0u8..3,
        ) {
            let sign = match sign {
                1 => "+",
                2 => "-",
                _ => "",
            };
            let zero = format!("{sign}{}", "0".repeat(zero_digits));
            let xml = format!(
                r#"<w:pPr xmlns:w="w"><w:ind w:left="{zero}" w:right="{zero}"/><w:spacing w:before="{zero}" w:after="{zero}"/></w:pPr>"#
            );
            let root = parse_docx_xml(xml.as_bytes()).unwrap();
            let mut properties = paragraph_properties(&root, &DocxTheme::default());
            properties.retain(|name, value| !is_default_paragraph_property(name, value));

            prop_assert!(properties.is_empty(), "{properties:?}");
        }

        #[test]
        fn DOCXの任意テーマ色と同じ直接色は等価になる(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            lowercase_theme in any::<bool>(),
        ) {
            let rgb = format!("{red:02X}{green:02X}{blue:02X}");
            let theme_rgb = if lowercase_theme {
                rgb.to_ascii_lowercase()
            } else {
                rgb.clone()
            };
            let theme_xml = format!(
                r#"<a:theme xmlns:a="a"><a:themeElements><a:clrScheme><a:accent1><a:srgbClr val="{theme_rgb}"/></a:accent1></a:clrScheme></a:themeElements></a:theme>"#
            );
            let theme = DocxTheme::from_root(
                &parse_docx_xml(theme_xml.as_bytes()).unwrap(),
            );
            let themed = parse_docx_xml(
                br#"<w:color xmlns:w="w" w:val="000000" w:themeColor="accent1"/>"#,
            )
            .unwrap();
            let direct_xml = format!(r#"<w:color xmlns:w="w" w:val="{rgb}"/>"#);
            let direct = parse_docx_xml(direct_xml.as_bytes()).unwrap();

            prop_assert_eq!(
                canonical_docx_node(&themed, Some(&theme)),
                canonical_docx_node(&direct, Some(&theme)),
            );
        }

        #[test]
        fn どんなUnicode入力でも完全に復元でき書記素境界を守る(
            old in unicode_text(),
            new in unicode_text(),
        ) {
            let segments = Segmenter::char_only().segments(&old, &new);
            assert_valid_segments(&old, &new, &segments)?;
        }
    }
}
