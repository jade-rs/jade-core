use crate::document::{
    BorderEdge, EastAsianLineBreakingSettings, FontHint, Indent, Numbering, ParagraphBorders,
    ParagraphProperties, RunFitText, RunFonts, RunProperties, Spacing, TabStop, TableBorders,
    TableCellMargins, TableStyleCondition, TableStyleConditionalFormat, TableStyleDisplay,
    TableStyleFirstRow,
};
use crate::numbering::NumberingPart;
use crate::parser::DocumentParser;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

type ParseResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
struct StyleParseState {
    in_supported_style: bool,
    in_paragraph_style: bool,
    in_character_style: bool,
    /// Inside a `w:style w:type="table"`, whose own `w:pPr` cell paragraphs
    /// inherit.
    in_table_style: bool,
    /// Inside a table style's `w:tblStylePr`, which applies to some cells only.
    in_table_style_conditional: bool,
    /// Inside a paragraph style's `w:pPr`, or a table style's own one.
    in_paragraph_properties: bool,
    in_numbering_properties: bool,
    in_paragraph_border: bool,
    in_tabs: bool,
    in_style_run_properties: bool,
    in_doc_defaults: bool,
    in_doc_default_run_properties: bool,
    in_doc_default_paragraph_properties: bool,
}

/// Parsed representation of `word/styles.xml`. This structure currently
/// focuses on paragraph styles so that numbering inherited from styles is
/// preserved even when a paragraph omits `<w:numPr>`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StylesPart {
    paragraph_styles: HashMap<String, StyleDefinition>,
    character_styles: HashMap<String, StyleDefinition>,
    /// Table styles, for the paragraph properties they give cell paragraphs.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    table_paragraph_styles: HashMap<String, StyleDefinition>,
    #[serde(skip)]
    table_cell_margin_styles: HashMap<String, TableCellMarginStyleDefinition>,
    table_styles: HashMap<String, TableStyleDefinition>,
    table_style_order: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_default_run_properties: Option<RunProperties>,
    /// `w:docDefaults/w:pPrDefault/w:pPr`: document-wide paragraph defaults
    /// (indent / spacing / alignment). Applied below every named style.
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_default_paragraph_properties: Option<ParagraphProperties>,
    /// Ids of the paragraph styles named `Normal`, in document order.
    #[serde(skip)]
    normal_named_paragraph_styles: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct TableCellMarginStyleDefinition {
    based_on: Option<String>,
    margins: Option<TableCellMargins>,
}

/// Public gallery entry for one package-local table style.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TableStyleInfo {
    pub style_id: String,
    pub name: String,
    pub display: TableStyleDisplay,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TableStyleDefinition {
    style_id: String,
    name: Option<String>,
    based_on: Option<String>,
    /// `w:default="1"`: the style Word applies to tables without `w:tblStyle`.
    is_default: bool,
    display: TableStyleDisplay,
}

#[allow(clippy::too_many_arguments)]
impl StylesPart {
    /// Creates an empty `StylesPart`. Useful when the package omits
    /// `word/styles.xml` but we still want a neutral default to avoid
    /// conditional logic downstream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses a style definition part from raw XML text.
    pub fn from_xml(xml: &str) -> ParseResult<Self> {
        let mut part = Self::new();
        part.parse(xml)?;
        Ok(part)
    }

    /// Parses style information and stores the result in `self`.
    pub fn parse(&mut self, xml: &str) -> ParseResult<()> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut buf = Vec::new();
        let mut current_style: Option<StyleDefinition> = None;
        let mut state = StyleParseState::default();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    self.handle_start_event(e, &mut current_style, &mut state);
                }
                Ok(Event::Empty(ref e)) => {
                    self.handle_start_event(e, &mut current_style, &mut state);
                    self.handle_end_event(e.name().as_ref(), &mut current_style, &mut state);
                }
                Ok(Event::End(ref e)) => {
                    self.handle_end_event(e.name().as_ref(), &mut current_style, &mut state);
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("Styles XML parsing error: {err}").into()),
                _ => {}
            }
            buf.clear();
        }

        self.merge_duplicate_normal_styles();
        self.parse_table_cell_margin_styles(xml)?;
        self.parse_table_styles(xml)?;
        Ok(())
    }

    /// Word keys its built-in Normal style by name: when several paragraph
    /// styles are named `Normal`, the last definition is the one Word uses,
    /// for paragraphs without a style and for every one of those style ids.
    fn merge_duplicate_normal_styles(&mut self) {
        let Some((last_id, earlier_ids)) = self.normal_named_paragraph_styles.split_last() else {
            return;
        };
        let Some(last) = self.paragraph_styles.get(last_id).cloned() else {
            return;
        };
        for id in earlier_ids.iter().filter(|id| *id != last_id) {
            let Some(style) = self.paragraph_styles.get_mut(id) else {
                continue;
            };
            *style = StyleDefinition {
                style_id: style.style_id.clone(),
                is_default: style.is_default,
                based_on: last.based_on.clone().filter(|based_on| based_on != id),
                ..last.clone()
            };
        }
    }

    /// Resolves `w:tblCellMar` through a table style's `w:basedOn` chain.
    /// Direct table formatting is applied later by `WordDocument`, after the
    /// package-local style has been materialized onto the parsed table model.
    pub fn resolve_table_cell_margins(&self, style_id: &str) -> Option<TableCellMargins> {
        self.resolve_table_cell_margins_by_id(style_id, &mut HashSet::new())
    }

    fn resolve_table_cell_margins_by_id(
        &self,
        style_id: &str,
        visited: &mut HashSet<String>,
    ) -> Option<TableCellMargins> {
        if !visited.insert(style_id.to_string()) {
            return None;
        }
        let definition = self.table_cell_margin_styles.get(style_id)?;
        let inherited = definition
            .based_on
            .as_deref()
            .and_then(|parent| self.resolve_table_cell_margins_by_id(parent, visited));
        match (inherited, definition.margins.clone()) {
            (Some(parent), Some(child)) => Some(merge_table_cell_margins(parent, child)),
            (parent, child) => child.or(parent),
        }
    }

    fn parse_table_cell_margin_styles(&mut self, xml: &str) -> ParseResult<()> {
        self.table_cell_margin_styles.clear();
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);
        let mut current: Option<(String, TableCellMarginStyleDefinition)> = None;
        let mut in_cell_margins = false;
        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref element)) => {
                    handle_table_cell_margin_style_start(
                        element,
                        &mut current,
                        &mut in_cell_margins,
                    );
                }
                Ok(Event::Empty(ref element)) => {
                    handle_table_cell_margin_style_start(
                        element,
                        &mut current,
                        &mut in_cell_margins,
                    );
                }
                Ok(Event::End(ref element)) => match element.name().as_ref() {
                    b"w:tblCellMar" => in_cell_margins = false,
                    b"w:style" => {
                        if let Some((style_id, definition)) = current.take() {
                            self.table_cell_margin_styles.insert(style_id, definition);
                        }
                        in_cell_margins = false;
                    }
                    _ => {}
                },
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("Styles XML parsing error: {err}").into()),
                _ => {}
            }
            buf.clear();
        }

        Ok(())
    }

    /// Resolves a table style token by exact id and then normalized display
    /// name. The returned id is always the package-local `w:styleId`.
    pub fn resolve_table_style_id<'a>(&'a self, requested: &'a str) -> Option<&'a str> {
        let trimmed = requested.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(style) = self.table_styles.get(trimmed) {
            return Some(style.style_id.as_str());
        }
        let requested_key = normalize_style_lookup_key(trimmed);
        self.table_style_order
            .iter()
            .filter_map(|style_id| self.table_styles.get(style_id))
            .find(|style| {
                normalize_style_lookup_key(&style.style_id) == requested_key
                    || style
                        .name
                        .as_deref()
                        .map(normalize_style_lookup_key)
                        .as_deref()
                        == Some(requested_key.as_str())
            })
            .map(|style| style.style_id.as_str())
    }

    /// Returns the id of the table style marked `w:default="1"` ("Normal
    /// Table" in Word's own packages). Word applies it to every table that
    /// carries no `w:tblStyle`; its `w:tblCellMar` (0 / 108 / 0 / 108 dxa) is
    /// what gives a Word-inserted table zero vertical cell padding.
    pub fn default_table_style_id(&self) -> Option<&str> {
        self.table_style_order
            .iter()
            .filter_map(|style_id| self.table_styles.get(style_id))
            .find(|style| style.is_default)
            .map(|style| style.style_id.as_str())
    }

    /// Returns the basedOn-resolved display values of the default table style.
    pub fn default_table_style(&self) -> Option<TableStyleDisplay> {
        let style_id = self.default_table_style_id()?;
        self.resolve_table_style_by_id(style_id, &mut HashSet::new())
            .map(|(_, display)| display)
    }

    /// Returns the basedOn-resolved display values for a table style.
    pub fn resolve_table_style(&self, requested: &str) -> Option<TableStyleDisplay> {
        let style_id = self.resolve_table_style_id(requested)?;
        self.resolve_table_style_by_id(style_id, &mut HashSet::new())
            .map(|(_, display)| display)
    }

    /// Enumerates table styles in styles.xml document order for a host gallery.
    /// Normal Table stays resolvable and the default for unstyled tables, but
    /// is not offered as a gallery entry.
    pub fn table_style_infos(&self) -> Vec<TableStyleInfo> {
        self.table_style_order
            .iter()
            .filter_map(|style_id| {
                let definition = self.table_styles.get(style_id)?;
                // Japanese Word localizes the built-in id (`a1`) but keeps the
                // English built-in name in `w:name`, so check both.
                let is_normal_table = std::iter::once(style_id.as_str())
                    .chain(definition.name.as_deref())
                    .map(normalize_style_lookup_key)
                    .any(|key| matches!(key.as_str(), "tablenormal" | "normaltable"));
                if is_normal_table {
                    return None;
                }
                let (_, display) = self.resolve_table_style_by_id(style_id, &mut HashSet::new())?;
                Some(TableStyleInfo {
                    style_id: style_id.clone(),
                    name: definition.name.clone().unwrap_or_else(|| style_id.clone()),
                    display,
                })
            })
            .collect()
    }

    fn resolve_table_style_by_id(
        &self,
        style_id: &str,
        visited: &mut HashSet<String>,
    ) -> Option<(String, TableStyleDisplay)> {
        if !visited.insert(style_id.to_string()) {
            return None;
        }
        let definition = self.table_styles.get(style_id)?;
        let mut display = definition.display.clone();
        if let Some(parent_id) = definition.based_on.as_deref() {
            if let Some((_, parent)) = self.resolve_table_style_by_id(parent_id, visited) {
                display = merge_table_style_display(parent, display);
            }
        }
        Some((style_id.to_string(), display))
    }

    /// Looks up numbering metadata attached to a paragraph style, following
    /// `basedOn` links until a numbering definition is found or the chain
    /// terminates.
    pub fn resolve_numbering(&self, style_id: &str) -> Option<Numbering> {
        let mut current = Some(style_id);
        let mut visited = HashSet::new();

        while let Some(id) = current {
            if !visited.insert(id) {
                break;
            }

            if let Some(style) = self.paragraph_styles.get(id) {
                if let Some(numbering) = style.numbering.clone() {
                    return Some(numbering);
                }
                current = style.based_on.as_deref();
            } else {
                break;
            }
        }

        None
    }

    /// Returns the paragraph style id Word treats as the default style.
    pub fn default_paragraph_style_id(&self) -> Option<&str> {
        self.paragraph_styles
            .values()
            .find(|style| style.is_default)
            .map(|style| style.style_id.as_str())
            .or_else(|| {
                self.paragraph_styles
                    .get("Normal")
                    .map(|style| style.style_id.as_str())
            })
    }

    /// Resolves a paragraph style token supplied by UI/API callers to the
    /// actual `w:styleId` used by the document. Word's localized built-in
    /// styles can use compact IDs such as `1` while their `<w:name>` remains
    /// `heading 1`, so callers should not have to know the package-local ID.
    pub fn resolve_paragraph_style_id<'a>(&'a self, requested: &'a str) -> Option<&'a str> {
        let trimmed = requested.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(style) = self.paragraph_styles.get(trimmed) {
            return Some(style.style_id.as_str());
        }

        let requested_key = normalize_style_lookup_key(trimmed);
        if requested_key.is_empty() {
            return None;
        }
        self.paragraph_styles
            .values()
            .find(|style| {
                normalize_style_lookup_key(&style.style_id) == requested_key
                    || style
                        .name
                        .as_deref()
                        .map(normalize_style_lookup_key)
                        .as_deref()
                        == Some(requested_key.as_str())
            })
            .map(|style| style.style_id.as_str())
    }

    /// Resolves a character style the same way [`Self::resolve_paragraph_style_id`]
    /// resolves a paragraph one: by `w:styleId` first, then by the normalized
    /// `w:name`.
    ///
    /// Callers usually hold what the gallery showed them, which is the display
    /// name; `w:styleId` is frequently an opaque token (`a3`).
    pub fn resolve_character_style_id<'a>(&'a self, requested: &'a str) -> Option<&'a str> {
        let trimmed = requested.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some(style) = self.character_styles.get(trimmed) {
            return Some(style.style_id.as_str());
        }

        let requested_key = normalize_style_lookup_key(trimmed);
        if requested_key.is_empty() {
            return None;
        }
        self.character_styles
            .values()
            .find(|style| {
                normalize_style_lookup_key(&style.style_id) == requested_key
                    || style
                        .name
                        .as_deref()
                        .map(normalize_style_lookup_key)
                        .as_deref()
                        == Some(requested_key.as_str())
            })
            .map(|style| style.style_id.as_str())
    }

    pub fn doc_default_font_size(&self) -> Option<f32> {
        self.doc_default_run_properties
            .as_ref()
            .and_then(|props| props.font_size)
    }

    pub fn doc_default_paragraph_properties(&self) -> Option<&ParagraphProperties> {
        self.doc_default_paragraph_properties.as_ref()
    }

    pub fn doc_default_run_properties(&self) -> Option<&RunProperties> {
        self.doc_default_run_properties.as_ref()
    }

    pub fn resolve_paragraph_properties(&self, style_id: &str) -> Option<ParagraphProperties> {
        Self::resolve_style_paragraph_properties(&self.paragraph_styles, style_id)
    }

    /// The paragraph properties (`w:pPr`, outside any `w:tblStylePr`) a table
    /// style gives the paragraphs in its cells, `w:basedOn` chain included.
    pub fn resolve_table_paragraph_properties(
        &self,
        style_id: &str,
    ) -> Option<ParagraphProperties> {
        Self::resolve_style_paragraph_properties(&self.table_paragraph_styles, style_id)
    }

    fn resolve_style_paragraph_properties(
        styles: &HashMap<String, StyleDefinition>,
        style_id: &str,
    ) -> Option<ParagraphProperties> {
        let mut current = Some(style_id);
        let mut visited = HashSet::new();
        let mut chain: Vec<&StyleDefinition> = Vec::new();

        while let Some(id) = current {
            if !visited.insert(id) {
                break;
            }
            if let Some(style) = styles.get(id) {
                chain.push(style);
                current = style.based_on.as_deref();
            } else {
                break;
            }
        }

        if chain.is_empty() {
            return None;
        }

        let mut resolved = ParagraphProperties::default();
        let mut has_any = false;
        for style in chain.iter().rev() {
            if let Some(paragraph_props) = style.paragraph_properties.as_ref() {
                merge_paragraph_properties_override(&mut resolved, paragraph_props);
                has_any = true;
            }
        }

        if has_any {
            Some(resolved)
        } else {
            None
        }
    }

    /// How many styles the part defines; a command that adds one changes it.
    pub fn style_count(&self) -> usize {
        self.paragraph_styles.len() + self.character_styles.len() + self.table_styles.len()
    }

    pub fn resolve_run_properties(&self, style_id: &str) -> Option<RunProperties> {
        self.resolve_style_run_properties(style_id, &self.paragraph_styles)
    }

    pub fn resolve_character_run_properties(&self, style_id: &str) -> Option<RunProperties> {
        self.resolve_style_run_properties(style_id, &self.character_styles)
    }

    /// Resolves the effective run properties for text in a paragraph using
    /// Word's style cascade: document defaults, paragraph style, character
    /// style, then direct run formatting. Later layers override earlier ones.
    pub fn resolve_effective_run_properties(
        &self,
        paragraph_style_id: Option<&str>,
        direct: Option<&RunProperties>,
    ) -> Option<RunProperties> {
        let mut resolved = RunProperties::default();
        let mut has_any = false;

        if let Some(defaults) = self.doc_default_run_properties() {
            merge_run_properties_override(&mut resolved, defaults);
            has_any = true;
        }

        let paragraph_style_id = paragraph_style_id.or_else(|| self.default_paragraph_style_id());
        if let Some(paragraph_props) =
            paragraph_style_id.and_then(|style_id| self.resolve_run_properties(style_id))
        {
            merge_run_properties_override(&mut resolved, &paragraph_props);
            has_any = true;
        }

        if let Some(character_props) = direct
            .and_then(|props| props.style.as_deref())
            .and_then(|style_id| self.resolve_character_run_properties(style_id))
        {
            merge_run_properties_override(&mut resolved, &character_props);
            has_any = true;
        }

        if let Some(direct) = direct {
            merge_run_properties_override(&mut resolved, direct);
            resolved.style = direct.style.clone();
            has_any = true;
        }

        has_any.then_some(resolved)
    }

    fn resolve_style_run_properties(
        &self,
        style_id: &str,
        styles: &HashMap<String, StyleDefinition>,
    ) -> Option<RunProperties> {
        const INLINE_CHAIN_CAPACITY: usize = 8;

        let mut current = Some(style_id);
        let mut inline_chain: [Option<&StyleDefinition>; INLINE_CHAIN_CAPACITY] =
            [None; INLINE_CHAIN_CAPACITY];
        let mut inline_len = 0;
        let mut overflow_chain = Vec::new();

        while let Some(id) = current {
            // Style inheritance chains are normally only a few entries long.
            // Reuse the inline chain for cycle detection instead of allocating
            // for every run whose formatting is resolved.
            if inline_chain[..inline_len]
                .iter()
                .flatten()
                .copied()
                .chain(overflow_chain.iter().copied())
                .any(|style| style.style_id == id)
            {
                break;
            }
            if let Some(style) = styles.get(id) {
                if inline_len < INLINE_CHAIN_CAPACITY {
                    inline_chain[inline_len] = Some(style);
                    inline_len += 1;
                } else {
                    overflow_chain.push(style);
                }
                current = style.based_on.as_deref();
            } else {
                break;
            }
        }

        if inline_len == 0 {
            return None;
        }

        let mut resolved = RunProperties::default();
        let mut has_any = false;
        for style in overflow_chain
            .iter()
            .rev()
            .copied()
            .chain(inline_chain[..inline_len].iter().rev().flatten().copied())
        {
            if let Some(run_props) = style.run_properties.as_ref() {
                merge_run_properties_override(&mut resolved, run_props);
                has_any = true;
            }
        }

        if has_any {
            Some(resolved)
        } else {
            None
        }
    }

    pub fn resolve_font_size(&self, style_id: &str) -> Option<f32> {
        let mut current = Some(style_id);
        let mut visited = HashSet::new();

        while let Some(id) = current {
            if !visited.insert(id) {
                break;
            }
            if let Some(style) = self.paragraph_styles.get(id) {
                if let Some(run_props) = style.run_properties.as_ref() {
                    if let Some(size) = run_props.font_size {
                        return Some(size);
                    }
                }
                current = style.based_on.as_deref();
            } else {
                break;
            }
        }

        None
    }

    /// Returns a style definition for debugging or higher-level inspection.
    pub fn get_style(&self, style_id: &str) -> Option<&StyleDefinition> {
        self.paragraph_styles.get(style_id)
    }

    /// Every paragraph style definition in the part.
    ///
    /// The resolve helpers above answer "what does this style mean"; callers
    /// that have to *name* a style instead — an agent choosing a `style_id` for
    /// `set_paragraph_style`, a UI listing the style gallery — need the
    /// catalogue itself. Order is unspecified (the backing map is unordered),
    /// so sort by whatever key the caller presents.
    pub fn paragraph_style_definitions(&self) -> impl Iterator<Item = &StyleDefinition> {
        self.paragraph_styles.values()
    }

    /// Every character (run) style definition in the part.
    pub fn character_style_definitions(&self) -> impl Iterator<Item = &StyleDefinition> {
        self.character_styles.values()
    }

    /// Character styles that belong in an end-user style gallery. Package
    /// implementation styles remain resolvable by id but are not offered as
    /// ordinary authoring choices.
    pub fn character_style_gallery_definitions(&self) -> impl Iterator<Item = &StyleDefinition> {
        self.character_styles.values().filter(|definition| {
            let gallery_key = normalize_style_lookup_key(&definition.style_id);
            let is_builtin_implementation_style = matches!(
                gallery_key.as_str(),
                "hyperlink" | "followedhyperlink" | "defaultparagraphfont"
            );
            let links_to_paragraph_style = definition
                .linked_style
                .as_deref()
                .is_some_and(|id| self.paragraph_styles.contains_key(id));
            let is_linked_from_paragraph_style = self
                .paragraph_styles
                .values()
                .any(|style| style.linked_style.as_deref() == Some(definition.style_id.as_str()));

            !definition.semi_hidden
                && !is_builtin_implementation_style
                && !links_to_paragraph_style
                && !is_linked_from_paragraph_style
        })
    }

    fn parse_table_styles(&mut self, xml: &str) -> ParseResult<()> {
        self.table_styles.clear();
        self.table_style_order.clear();
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);
        let mut state = TableStyleParseState::default();
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref element)) => {
                    handle_table_style_start(element, &mut state);
                    state.path.push(element.name().as_ref().to_vec());
                }
                Ok(Event::Empty(ref element)) => {
                    handle_table_style_start(element, &mut state);
                    finish_table_style_element(
                        element.name().as_ref(),
                        &mut state,
                        &mut self.table_styles,
                        &mut self.table_style_order,
                    );
                }
                Ok(Event::End(ref element)) => {
                    finish_table_style_element(
                        element.name().as_ref(),
                        &mut state,
                        &mut self.table_styles,
                        &mut self.table_style_order,
                    );
                    state.path.pop();
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("Table styles XML parsing error: {err}").into()),
                _ => {}
            }
            buf.clear();
        }
        Ok(())
    }

    /// Merges numbering information exposed via `numbering.xml` (`w:pStyle`
    /// entries on numbering levels) into the cached style definitions. This
    /// mirrors docxjs の `prodessNumberings` と同じ役割で、スタイル定義に
    /// `<w:numPr>` が含まれない場合でも段落スタイルに番号付けを付与できる。
    pub fn merge_numbering_from_part(&mut self, numbering_part: &NumberingPart) {
        for instance in numbering_part.numbering_instances.values() {
            if let Some(abstract_num) = numbering_part.abstract_nums.get(&instance.abstract_num_id)
            {
                for level in &abstract_num.levels {
                    if let Some(style_id) = &level.paragraph_style {
                        let style_entry = self
                            .paragraph_styles
                            .entry(style_id.clone())
                            .or_insert_with(|| StyleDefinition {
                                style_id: style_id.clone(),
                                name: None,
                                ..StyleDefinition::default()
                            });
                        let numbering = style_entry.numbering.get_or_insert(Numbering {
                            id: None,
                            level: None,
                        });

                        if numbering.id.is_none() {
                            numbering.id = Some(instance.id.clone());
                        }
                        if numbering.level.is_none() {
                            numbering.level = Some(level.ilvl);
                        }

                        // Numbering 定義側で段落/ランの既定スタイルが指定されている場合、
                        // スタイルに既に情報が無ければ補完しておくと docxjs と同様の見た目になる。
                        if numbering.level.is_some() {
                            if style_entry.paragraph_properties.is_none()
                                && level.paragraph_props.is_some()
                            {
                                style_entry.paragraph_properties = level.paragraph_props.clone();
                            }
                            if style_entry.run_properties.is_none() && level.run_props.is_some() {
                                style_entry.run_properties = level.run_props.clone();
                            }
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_start_event(
        &mut self,
        element: &BytesStart,
        current_style: &mut Option<StyleDefinition>,
        state: &mut StyleParseState,
    ) {
        match element.name().as_ref() {
            b"w:style" => {
                let style_type = attr_value(element, b"w:type");
                let style_id = attr_value(element, b"w:styleId");
                let is_default = attr_value(element, b"w:default")
                    .as_deref()
                    .map(parse_on_off_value)
                    .unwrap_or(false);
                state.in_supported_style = matches!(
                    style_type.as_deref(),
                    Some("paragraph") | Some("character") | Some("table")
                );
                state.in_paragraph_style = matches!(style_type.as_deref(), Some("paragraph"));
                state.in_character_style = matches!(style_type.as_deref(), Some("character"));
                state.in_table_style = matches!(style_type.as_deref(), Some("table"));
                state.in_table_style_conditional = false;
                state.in_paragraph_properties = false;
                state.in_numbering_properties = false;
                state.in_paragraph_border = false;
                state.in_tabs = false;
                state.in_style_run_properties = false;

                if state.in_supported_style {
                    if let Some(id) = style_id {
                        *current_style = Some(StyleDefinition {
                            style_id: id,
                            name: None,
                            based_on: None,
                            next_style: None,
                            linked_style: None,
                            semi_hidden: false,
                            is_default,
                            numbering: None,
                            paragraph_properties: None,
                            run_properties: None,
                        });
                    } else {
                        *current_style = None;
                        state.in_supported_style = false;
                        state.in_paragraph_style = false;
                        state.in_character_style = false;
                        state.in_table_style = false;
                    }
                } else {
                    *current_style = None;
                }
            }
            b"w:tblStylePr" => {
                state.in_table_style_conditional = state.in_table_style;
            }
            b"w:basedOn" => {
                if state.in_supported_style {
                    if let Some(style) = current_style.as_mut() {
                        style.based_on = attr_value(element, b"w:val");
                    }
                }
            }
            b"w:link" => {
                if state.in_supported_style {
                    if let Some(style) = current_style.as_mut() {
                        style.linked_style = attr_value(element, b"w:val");
                    }
                }
            }
            b"w:next" => {
                if state.in_supported_style {
                    if let Some(style) = current_style.as_mut() {
                        style.next_style = attr_value(element, b"w:val");
                    }
                }
            }
            b"w:semiHidden" => {
                if state.in_supported_style {
                    if let Some(style) = current_style.as_mut() {
                        style.semi_hidden = parse_on_off_attr(element, b"w:val");
                    }
                }
            }
            b"w:name" => {
                if state.in_supported_style {
                    if let Some(style) = current_style.as_mut() {
                        style.name = attr_value(element, b"w:val");
                    }
                }
            }
            b"w:pPr" => {
                if state.in_paragraph_style
                    || (state.in_table_style && !state.in_table_style_conditional)
                {
                    state.in_paragraph_properties = true;
                } else if state.in_doc_defaults {
                    state.in_doc_default_paragraph_properties = true;
                    self.doc_default_paragraph_properties
                        .get_or_insert_with(ParagraphProperties::default);
                }
                state.in_tabs = false;
                state.in_style_run_properties = false;
            }
            b"w:rPr" => {
                if state.in_supported_style
                    && !state.in_table_style
                    && !state.in_paragraph_properties
                {
                    state.in_style_run_properties = true;
                    if let Some(style) = current_style.as_mut() {
                        style
                            .run_properties
                            .get_or_insert_with(RunProperties::default);
                    }
                } else if state.in_doc_defaults {
                    state.in_doc_default_run_properties = true;
                    self.doc_default_run_properties
                        .get_or_insert_with(RunProperties::default);
                }
            }
            b"w:numPr" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        ensure_numbering(style);
                    }
                    state.in_numbering_properties = true;
                }
            }
            b"w:numId" => {
                if state.in_numbering_properties {
                    if let Some(style) = current_style.as_mut() {
                        if let Some(numbering) = style.numbering.as_mut() {
                            numbering.id = attr_value(element, b"w:val");
                        }
                    }
                }
            }
            b"w:ilvl" => {
                if state.in_numbering_properties {
                    if let Some(style) = current_style.as_mut() {
                        if let Some(numbering) = style.numbering.as_mut() {
                            numbering.level = attr_value(element, b"w:val")
                                .and_then(|value| value.parse::<u32>().ok());
                        }
                    }
                }
            }
            b"w:tabs" => {
                if state.in_paragraph_properties {
                    state.in_tabs = true;
                }
            }
            b"w:jc" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        props.alignment = attr_value(element, b"w:val").map(Into::into);
                    }
                } else if state.in_doc_default_paragraph_properties {
                    if let Some(props) = self.doc_default_paragraph_properties.as_mut() {
                        props.alignment = attr_value(element, b"w:val").map(Into::into);
                    }
                }
            }
            b"w:bidi" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        props.bidi = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:spacing"
                if state.in_style_run_properties || state.in_doc_default_run_properties =>
            {
                // Character spacing (`w:rPr/w:spacing`, ECMA-376 §17.3.2.35),
                // in twentieths of a point, as direct run formatting reads it.
                if let Some(twips) =
                    attr_value(element, b"w:val").and_then(|value| value.parse::<f32>().ok())
                {
                    apply_to_active_run_properties(self, current_style, state, |props| {
                        props.char_spacing = Some(twips / 20.0);
                    });
                }
            }
            b"w:spacing" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        if let Some(spacing) = parse_spacing_definition(element) {
                            let props = style
                                .paragraph_properties
                                .get_or_insert_with(ParagraphProperties::default);
                            props.spacing = Some(spacing);
                        }
                    }
                } else if state.in_doc_default_paragraph_properties {
                    if let Some(spacing) = parse_spacing_definition(element) {
                        if let Some(props) = self.doc_default_paragraph_properties.as_mut() {
                            props.spacing = Some(spacing);
                        }
                    }
                }
            }
            b"w:ind" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        if let Some(indent) = parse_indent_definition(element) {
                            let props = style
                                .paragraph_properties
                                .get_or_insert_with(ParagraphProperties::default);
                            props.indent = Some(indent);
                        }
                    }
                } else if state.in_doc_default_paragraph_properties {
                    if let Some(indent) = parse_indent_definition(element) {
                        if let Some(props) = self.doc_default_paragraph_properties.as_mut() {
                            props.indent = Some(indent);
                        }
                    }
                }
            }
            b"w:shd" => {
                // Paragraph shading carried by a style (ECMA-376 §17.3.5). Only
                // honor `w:shd` directly under the paragraph `w:pPr`, not the one
                // nested inside `w:rPr` (run shading) which we do not model here.
                if state.in_paragraph_properties && !state.in_style_run_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        props.shading = Some(DocumentParser::parse_shading(element.attributes()));
                    }
                }
            }
            b"w:pBdr" => {
                // Paragraph borders carried by a style (ECMA-376 §17.3.1.24).
                // The edge children (`w:top`/`w:bottom`/...) arrive as separate
                // events, so flag that we are inside a `w:pBdr` and ensure the
                // borders container exists.
                if state.in_paragraph_properties {
                    state.in_paragraph_border = true;
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        props.borders.get_or_insert_with(ParagraphBorders::default);
                    }
                }
            }
            b"w:top" | b"w:bottom" | b"w:left" | b"w:right" | b"w:between" | b"w:bar"
                if state.in_paragraph_border =>
            {
                if let Some(style) = current_style.as_mut() {
                    if let Some(borders) = style
                        .paragraph_properties
                        .as_mut()
                        .and_then(|props| props.borders.as_mut())
                    {
                        let edge = DocumentParser::build_border_edge(element.attributes());
                        match element.name().as_ref() {
                            b"w:top" => borders.top = Some(edge),
                            b"w:bottom" => borders.bottom = Some(edge),
                            b"w:left" => borders.left = Some(edge),
                            b"w:right" => borders.right = Some(edge),
                            b"w:between" => borders.between = Some(edge),
                            b"w:bar" => borders.bar = Some(edge),
                            _ => {}
                        }
                    }
                }
            }
            b"w:tab" => {
                if state.in_paragraph_properties && state.in_tabs {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        if let Some(tab) = parse_tab_definition(element) {
                            props.tabs.push(tab);
                        }
                    }
                }
            }
            b"w:autoSpaceDE" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        let east_asian = props
                            .east_asian_settings
                            .get_or_insert_with(EastAsianLineBreakingSettings::default);
                        east_asian.use_east_asian_layout = true;
                        east_asian.auto_space_de = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:autoSpaceDN" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        let east_asian = props
                            .east_asian_settings
                            .get_or_insert_with(EastAsianLineBreakingSettings::default);
                        east_asian.use_east_asian_layout = true;
                        east_asian.auto_space_dn = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:kinsoku" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        let east_asian = props
                            .east_asian_settings
                            .get_or_insert_with(EastAsianLineBreakingSettings::default);
                        east_asian.use_east_asian_layout = true;
                        east_asian.kinsoku = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:wordWrap" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        let east_asian = props
                            .east_asian_settings
                            .get_or_insert_with(EastAsianLineBreakingSettings::default);
                        east_asian.use_east_asian_layout = true;
                        east_asian.word_wrap = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:overflowPunct" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        let east_asian = props
                            .east_asian_settings
                            .get_or_insert_with(EastAsianLineBreakingSettings::default);
                        east_asian.use_east_asian_layout = true;
                        east_asian.overflow_punctuation =
                            Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:topLinePunct" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        let east_asian = props
                            .east_asian_settings
                            .get_or_insert_with(EastAsianLineBreakingSettings::default);
                        east_asian.use_east_asian_layout = true;
                        east_asian.top_line_punctuation =
                            Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:adjustRightInd" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        props.adjust_right_indent = Some(parse_on_off_attr(element, b"w:val"));
                    }
                } else if state.in_doc_default_paragraph_properties {
                    if let Some(props) = self.doc_default_paragraph_properties.as_mut() {
                        props.adjust_right_indent = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:snapToGrid" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        props.snap_to_grid = Some(parse_on_off_attr(element, b"w:val"));
                    }
                } else if state.in_doc_default_paragraph_properties {
                    if let Some(props) = self.doc_default_paragraph_properties.as_mut() {
                        props.snap_to_grid = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:widowControl" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        props.widow_control = Some(parse_on_off_attr(element, b"w:val"));
                    }
                } else if state.in_doc_default_paragraph_properties {
                    if let Some(props) = self.doc_default_paragraph_properties.as_mut() {
                        props.widow_control = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:suppressAutoHyphens" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        props.suppress_auto_hyphens = Some(parse_on_off_attr(element, b"w:val"));
                    }
                } else if state.in_doc_default_paragraph_properties {
                    if let Some(props) = self.doc_default_paragraph_properties.as_mut() {
                        props.suppress_auto_hyphens = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:contextualSpacing" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        props.contextual_spacing = Some(parse_on_off_attr(element, b"w:val"));
                    }
                } else if state.in_doc_default_paragraph_properties {
                    if let Some(props) = self.doc_default_paragraph_properties.as_mut() {
                        props.contextual_spacing = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:framePr" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let attributes = element
                            .attributes()
                            .flatten()
                            .filter_map(|attr| {
                                attr.key.as_ref().strip_prefix(b"w:").map(|local| {
                                    (
                                        String::from_utf8_lossy(local).into_owned(),
                                        String::from_utf8_lossy(&attr.value).into_owned(),
                                    )
                                })
                            })
                            .collect::<Vec<_>>();
                        if let Some(frame) =
                            crate::document::ParagraphFrame::from_attributes(|name| {
                                attributes
                                    .iter()
                                    .find(|(key, _)| key == name)
                                    .map(|(_, value)| value.as_str())
                            })
                        {
                            style
                                .paragraph_properties
                                .get_or_insert_with(ParagraphProperties::default)
                                .frame = Some(frame);
                        }
                    }
                }
            }
            b"w:pageBreakBefore" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        props.page_break_before = Some(parse_on_off_attr(element, b"w:val"));
                    }
                } else if state.in_doc_default_paragraph_properties {
                    if let Some(props) = self.doc_default_paragraph_properties.as_mut() {
                        props.page_break_before = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:suppressLineNumbers" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        props.suppress_line_numbers = Some(parse_on_off_attr(element, b"w:val"));
                    }
                } else if state.in_doc_default_paragraph_properties {
                    if let Some(props) = self.doc_default_paragraph_properties.as_mut() {
                        props.suppress_line_numbers = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:keepLines" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        props.keep_lines = Some(parse_on_off_attr(element, b"w:val"));
                    }
                } else if state.in_doc_default_paragraph_properties {
                    if let Some(props) = self.doc_default_paragraph_properties.as_mut() {
                        props.keep_lines = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:keepNext" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        props.keep_next = Some(parse_on_off_attr(element, b"w:val"));
                    }
                } else if state.in_doc_default_paragraph_properties {
                    if let Some(props) = self.doc_default_paragraph_properties.as_mut() {
                        props.keep_next = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:textAlignment" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        if let Some(token) = attr_value(element, b"w:val") {
                            let token = token.trim().to_ascii_lowercase();
                            if !token.is_empty() {
                                let props = style
                                    .paragraph_properties
                                    .get_or_insert_with(ParagraphProperties::default);
                                props.text_alignment = Some(token.into());
                            }
                        }
                    }
                }
            }
            b"w:mirrorIndents" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        let props = style
                            .paragraph_properties
                            .get_or_insert_with(ParagraphProperties::default);
                        props.mirror_indents = Some(parse_on_off_attr(element, b"w:val"));
                    }
                }
            }
            b"w:outlineLvl" => {
                if state.in_paragraph_properties {
                    if let Some(style) = current_style.as_mut() {
                        if let Some(level) = attr_value(element, b"w:val")
                            .and_then(|raw| raw.trim().parse::<i64>().ok())
                        {
                            let props = style
                                .paragraph_properties
                                .get_or_insert_with(ParagraphProperties::default);
                            props.outline_level = Some(level.clamp(0, 9) as u8);
                        }
                    }
                }
            }
            b"w:docDefaults" => {
                state.in_doc_defaults = true;
            }
            b"w:rPrDefault" => {
                if state.in_doc_defaults {
                    self.doc_default_run_properties
                        .get_or_insert_with(RunProperties::default);
                }
            }
            b"w:b" | b"w:i" | b"w:strike" => {
                let value = parse_on_off_attr(element, b"w:val");
                apply_to_active_run_properties(self, current_style, state, |props| {
                    match element.name().as_ref() {
                        b"w:b" => props.bold = Some(value),
                        b"w:i" => props.italic = Some(value),
                        b"w:strike" => props.strike = Some(value),
                        _ => unreachable!(),
                    }
                });
            }
            b"w:u" => {
                // [MS-OE376 §2.3.48](https://learn.microsoft.com/openspecs/office_standards/ms-oe376/70728747-cf2d-4393-afb3-f5117faa27d5):
                // when `w:val` is absent Word does not treat `w:u` as shorthand
                // for `single`. It consults the style hierarchy and, if no
                // value is found, renders no underline. Leave the property
                // unspecified so the cascade can do that.
                if let Some(raw) = attr_value(element, b"w:val") {
                    let value = parse_underline_value(&raw);
                    let kind =
                        (value && raw != "single" && raw != "1" && raw != "true" && raw != "on")
                            .then_some(raw);
                    let color = attr_value(element, b"w:color").filter(|value| !value.is_empty());
                    apply_to_active_run_properties(self, current_style, state, |props| {
                        props.underline = Some(value);
                        props.underline_kind = kind.as_deref().map(Into::into);
                        props.underline_color = color.as_deref().map(Into::into);
                    });
                }
            }
            b"w:color" => {
                // A theme colour in the scheme being read (see
                // `theme::ThemeColorScope`), else the cached value.
                let color = attr_value(element, b"w:themeColor")
                    .and_then(|theme| {
                        crate::theme::text_theme_color(
                            &theme,
                            attr_value(element, b"w:themeTint").as_deref(),
                            attr_value(element, b"w:themeShade").as_deref(),
                        )
                    })
                    .or_else(|| attr_value(element, b"w:val").and_then(normalize_ooxml_color));
                let theme = attr_value(element, b"w:themeColor")
                    .filter(|name| !name.is_empty())
                    .map(|name| crate::document::RunThemeColor {
                        name: name.as_str().into(),
                        tint: attr_value(element, b"w:themeTint").map(|v| v.as_str().into()),
                        shade: attr_value(element, b"w:themeShade").map(|v| v.as_str().into()),
                    });
                apply_to_active_run_properties(self, current_style, state, |props| {
                    props.color = color.map(Into::into);
                    props.theme_color = theme.clone().map(Box::new);
                });
            }
            b"w:sz" => {
                if let Some(size) = parse_font_size(element) {
                    if state.in_style_run_properties {
                        if let Some(style) = current_style.as_mut() {
                            if let Some(run_props) = style.run_properties.as_mut() {
                                run_props.font_size = Some(size);
                            }
                        }
                    } else if state.in_doc_default_run_properties {
                        if let Some(defaults) = self.doc_default_run_properties.as_mut() {
                            defaults.font_size = Some(size);
                        }
                    }
                }
            }
            b"w:szCs" => {
                if let Some(size) = parse_font_size(element) {
                    if state.in_style_run_properties {
                        if let Some(style) = current_style.as_mut() {
                            if let Some(run_props) = style.run_properties.as_mut() {
                                run_props.font_size_complex = Some(size);
                            }
                        }
                    } else if state.in_doc_default_run_properties {
                        if let Some(defaults) = self.doc_default_run_properties.as_mut() {
                            defaults.font_size_complex = Some(size);
                        }
                    }
                }
            }
            b"w:kern" => {
                if let Some(points) = parse_kerning_points(element) {
                    if state.in_style_run_properties {
                        if let Some(style) = current_style.as_mut() {
                            if let Some(run_props) = style.run_properties.as_mut() {
                                run_props.kerning = Some(points);
                            }
                        }
                    } else if state.in_doc_default_run_properties {
                        if let Some(defaults) = self.doc_default_run_properties.as_mut() {
                            defaults.kerning = Some(points);
                        }
                    }
                }
            }
            b"w:w" => {
                if let Some(percent) = parse_text_scale_percent(element) {
                    if state.in_style_run_properties {
                        if let Some(style) = current_style.as_mut() {
                            if let Some(run_props) = style.run_properties.as_mut() {
                                run_props.text_scale = Some(percent);
                            }
                        }
                    } else if state.in_doc_default_run_properties {
                        if let Some(defaults) = self.doc_default_run_properties.as_mut() {
                            defaults.text_scale = Some(percent);
                        }
                    }
                }
            }
            b"w:fitText" => {
                if let Some(fit_text) = parse_fit_text(element) {
                    if state.in_style_run_properties {
                        if let Some(style) = current_style.as_mut() {
                            if let Some(run_props) = style.run_properties.as_mut() {
                                run_props.fit_text = Some(fit_text);
                            }
                        }
                    } else if state.in_doc_default_run_properties {
                        if let Some(defaults) = self.doc_default_run_properties.as_mut() {
                            defaults.fit_text = Some(fit_text);
                        }
                    }
                }
            }
            b"w:rFonts" => {
                if state.in_style_run_properties {
                    if let Some(style) = current_style.as_mut() {
                        if let Some(run_props) = style.run_properties.as_mut() {
                            apply_run_fonts(run_props, element);
                        }
                    }
                } else if state.in_doc_default_run_properties {
                    if let Some(defaults) = self.doc_default_run_properties.as_mut() {
                        apply_run_fonts(defaults, element);
                    }
                }
            }
            b"w:lang" => {
                let language = attr_value(element, b"w:val").map(Rc::from);
                let east_asia_language = attr_value(element, b"w:eastAsia")
                    .or_else(|| attr_value(element, b"eastAsia"))
                    .map(Rc::from);
                apply_to_active_run_properties(self, current_style, state, |props| {
                    props.language = language.clone();
                    props.east_asia_language = east_asia_language.clone();
                });
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_end_event(
        &mut self,
        name: &[u8],
        current_style: &mut Option<StyleDefinition>,
        state: &mut StyleParseState,
    ) {
        match name {
            b"w:style" => {
                if let Some(style) = current_style.take() {
                    if !style.style_id.is_empty() {
                        if state.in_paragraph_style {
                            if style
                                .name
                                .as_deref()
                                .is_some_and(|name| name.eq_ignore_ascii_case("Normal"))
                            {
                                self.normal_named_paragraph_styles
                                    .push(style.style_id.clone());
                            }
                            self.paragraph_styles.insert(style.style_id.clone(), style);
                        } else if state.in_character_style {
                            self.character_styles.insert(style.style_id.clone(), style);
                        } else if state.in_table_style {
                            self.table_paragraph_styles
                                .insert(style.style_id.clone(), style);
                        }
                    }
                }
                state.in_supported_style = false;
                state.in_paragraph_style = false;
                state.in_character_style = false;
                state.in_table_style = false;
                state.in_table_style_conditional = false;
                state.in_paragraph_properties = false;
                state.in_numbering_properties = false;
                state.in_paragraph_border = false;
                state.in_tabs = false;
                state.in_style_run_properties = false;
            }
            b"w:pPr" => {
                state.in_paragraph_properties = false;
                state.in_numbering_properties = false;
                state.in_paragraph_border = false;
                state.in_tabs = false;
                state.in_doc_default_paragraph_properties = false;
            }
            b"w:pBdr" => {
                state.in_paragraph_border = false;
            }
            b"w:tblStylePr" => {
                state.in_table_style_conditional = false;
            }
            b"w:rPr" => {
                state.in_style_run_properties = false;
                state.in_doc_default_run_properties = false;
            }
            b"w:numPr" => {
                state.in_numbering_properties = false;
            }
            b"w:tabs" => {
                state.in_tabs = false;
            }
            b"w:docDefaults" => {
                state.in_doc_defaults = false;
                state.in_doc_default_run_properties = false;
                state.in_doc_default_paragraph_properties = false;
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct TableStyleParseState {
    current: Option<TableStyleDefinition>,
    condition: Option<String>,
    path: Vec<Vec<u8>>,
}

fn table_style_path_contains(path: &[Vec<u8>], name: &[u8]) -> bool {
    path.iter().any(|entry| entry.as_slice() == name)
}

fn normalize_ooxml_color(value: String) -> Option<String> {
    if value.eq_ignore_ascii_case("auto") || value.is_empty() {
        None
    } else if value.starts_with('#') {
        Some(value.to_ascii_uppercase())
    } else if value.len() == 6 && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Some(format!("#{}", value.to_ascii_uppercase()))
    } else {
        Some(value)
    }
}

fn first_row_display(display: &mut TableStyleDisplay) -> &mut TableStyleFirstRow {
    display
        .first_row
        .get_or_insert_with(TableStyleFirstRow::default)
}

/// The conditional format being parsed, when inside a `w:tblStylePr` that
/// Word applies (`wholeTable` and unknown types yield `None`).
fn active_conditional_format(
    state: &mut TableStyleParseState,
) -> Option<&mut TableStyleConditionalFormat> {
    let condition = TableStyleCondition::from_type_token(state.condition.as_deref()?)?;
    Some(
        state
            .current
            .as_mut()?
            .display
            .conditional
            .entry(condition)
            .or_default(),
    )
}

fn set_table_style_fill(state: &mut TableStyleParseState, fill: String) {
    let Some(fill) = normalize_ooxml_color(fill) else {
        return;
    };
    if let Some(format) = active_conditional_format(state) {
        format.fill = Some(fill.clone());
    }
    let Some(style) = state.current.as_mut() else {
        return;
    };
    match state.condition.as_deref() {
        Some("firstRow") => first_row_display(&mut style.display).fill = Some(fill),
        Some("band1Horz") => style.display.band1_fill = Some(fill),
        Some("band2Horz") => style.display.band2_fill = Some(fill),
        Some(_) => {}
        None => style.display.fill = Some(fill),
    }
}

fn set_table_style_border(display: &mut TableStyleDisplay, element: &BytesStart) {
    set_table_borders_edge(
        display.borders.get_or_insert_with(TableBorders::default),
        element,
    );
}

fn set_table_borders_edge(borders: &mut TableBorders, element: &BytesStart) {
    let edge = BorderEdge {
        val: attr_value(element, b"w:val").map(|value| value.to_ascii_lowercase()),
        size_eighths: attr_value(element, b"w:sz").and_then(|value| value.parse().ok()),
        space: attr_value(element, b"w:space").and_then(|value| value.parse().ok()),
        color: attr_value(element, b"w:color").and_then(normalize_ooxml_color),
    };
    match element.name().as_ref() {
        b"w:top" => borders.top = Some(edge),
        b"w:bottom" => borders.bottom = Some(edge),
        b"w:left" => borders.left = Some(edge),
        b"w:right" => borders.right = Some(edge),
        b"w:start" => borders.start = Some(edge),
        b"w:end" => borders.end = Some(edge),
        b"w:insideH" => borders.inside_h = Some(edge),
        b"w:insideV" => borders.inside_v = Some(edge),
        _ => {}
    }
}

fn set_table_style_margin(display: &mut TableStyleDisplay, element: &BytesStart) {
    let width_type = attr_value(element, b"w:type").unwrap_or_else(|| "dxa".to_string());
    if width_type != "dxa" {
        return;
    }
    let Some(points) = attr_value(element, b"w:w")
        .and_then(|value| value.parse::<f32>().ok())
        .map(|twips| twips / 20.0)
    else {
        return;
    };
    let margins = display
        .cell_margins
        .get_or_insert_with(TableCellMargins::default);
    match element.name().as_ref() {
        b"w:top" => margins.top = Some(points),
        b"w:bottom" => margins.bottom = Some(points),
        b"w:left" => margins.left = Some(points),
        b"w:right" => margins.right = Some(points),
        b"w:start" => margins.start = Some(points),
        b"w:end" => margins.end = Some(points),
        _ => {}
    }
}

fn handle_table_style_start(element: &BytesStart, state: &mut TableStyleParseState) {
    match element.name().as_ref() {
        b"w:style" => {
            state.condition = None;
            state.current = if attr_value(element, b"w:type").as_deref() == Some("table") {
                let is_default = attr_value(element, b"w:default")
                    .as_deref()
                    .map(parse_on_off_value)
                    .unwrap_or(false);
                attr_value(element, b"w:styleId").map(|style_id| TableStyleDefinition {
                    style_id,
                    is_default,
                    ..TableStyleDefinition::default()
                })
            } else {
                None
            };
        }
        b"w:name" => {
            if let Some(style) = state.current.as_mut() {
                style.name = attr_value(element, b"w:val");
            }
        }
        b"w:basedOn" => {
            if let Some(style) = state.current.as_mut() {
                style.based_on = attr_value(element, b"w:val");
            }
        }
        b"w:tblStylePr" => {
            if state.current.is_some() {
                state.condition = attr_value(element, b"w:type");
            }
        }
        b"w:shd" if table_style_path_contains(&state.path, b"w:tcPr") => {
            if let Some(fill) = attr_value(element, b"w:fill") {
                set_table_style_fill(state, fill);
            }
        }
        b"w:b" if state.condition.is_some() && table_style_path_contains(&state.path, b"w:rPr") => {
            let bold = parse_on_off_attr(element, b"w:val");
            if let Some(format) = active_conditional_format(state) {
                format.bold = Some(bold);
            }
            if state.condition.as_deref() == Some("firstRow") {
                if let Some(style) = state.current.as_mut() {
                    first_row_display(&mut style.display).bold = Some(bold);
                }
            }
        }
        b"w:i" if state.condition.is_some() && table_style_path_contains(&state.path, b"w:rPr") => {
            let italic = parse_on_off_attr(element, b"w:val");
            if let Some(format) = active_conditional_format(state) {
                format.italic = Some(italic);
            }
        }
        b"w:color"
            if state.condition.is_some() && table_style_path_contains(&state.path, b"w:rPr") =>
        {
            if let Some(color) = attr_value(element, b"w:val").and_then(normalize_ooxml_color) {
                if let Some(format) = active_conditional_format(state) {
                    format.color = Some(color.clone());
                }
                if state.condition.as_deref() == Some("firstRow") {
                    if let Some(style) = state.current.as_mut() {
                        first_row_display(&mut style.display).color = Some(color);
                    }
                }
            }
        }
        b"w:top" | b"w:bottom" | b"w:left" | b"w:right" | b"w:start" | b"w:end" | b"w:insideH"
        | b"w:insideV"
            if state.condition.is_some()
                && table_style_path_contains(&state.path, b"w:tcBorders") =>
        {
            if let Some(format) = active_conditional_format(state) {
                set_table_borders_edge(
                    format.borders.get_or_insert_with(TableBorders::default),
                    element,
                );
            }
        }
        b"w:tblStyleRowBandSize" | b"w:tblStyleColBandSize" if state.condition.is_none() => {
            let size = attr_value(element, b"w:val").and_then(|value| value.parse::<u32>().ok());
            if let (Some(style), Some(size)) = (state.current.as_mut(), size) {
                if element.name().as_ref() == b"w:tblStyleRowBandSize" {
                    style.display.row_band_size = Some(size);
                } else {
                    style.display.col_band_size = Some(size);
                }
            }
        }
        b"w:top" | b"w:bottom" | b"w:left" | b"w:right" | b"w:start" | b"w:end" | b"w:insideH"
        | b"w:insideV"
            if state.condition.is_none()
                && table_style_path_contains(&state.path, b"w:tblBorders") =>
        {
            if let Some(style) = state.current.as_mut() {
                set_table_style_border(&mut style.display, element);
            }
        }
        b"w:top" | b"w:bottom" | b"w:left" | b"w:right" | b"w:start" | b"w:end"
            if state.condition.is_none()
                && table_style_path_contains(&state.path, b"w:tblCellMar") =>
        {
            if let Some(style) = state.current.as_mut() {
                set_table_style_margin(&mut style.display, element);
            }
        }
        _ => {}
    }
}

fn finish_table_style_element(
    name: &[u8],
    state: &mut TableStyleParseState,
    styles: &mut HashMap<String, TableStyleDefinition>,
    order: &mut Vec<String>,
) {
    match name {
        b"w:tblStylePr" => state.condition = None,
        b"w:style" => {
            if let Some(style) = state.current.take() {
                if !styles.contains_key(&style.style_id) {
                    order.push(style.style_id.clone());
                }
                styles.insert(style.style_id.clone(), style);
            }
            state.condition = None;
        }
        _ => {}
    }
}

fn merge_table_borders(parent: TableBorders, child: TableBorders) -> TableBorders {
    TableBorders::inherit(parent, child)
}

fn merge_table_margins(parent: TableCellMargins, child: TableCellMargins) -> TableCellMargins {
    TableCellMargins {
        top: child.top.or(parent.top),
        bottom: child.bottom.or(parent.bottom),
        left: child.left.or(parent.left),
        right: child.right.or(parent.right),
        start: child.start.or(parent.start),
        end: child.end.or(parent.end),
    }
}

fn merge_table_style_display(
    parent: TableStyleDisplay,
    child: TableStyleDisplay,
) -> TableStyleDisplay {
    let first_row = match (parent.first_row, child.first_row) {
        (Some(parent), Some(child)) => Some(TableStyleFirstRow {
            fill: child.fill.or(parent.fill),
            bold: child.bold.or(parent.bold),
            color: child.color.or(parent.color),
        }),
        (parent, child) => child.or(parent),
    };
    let mut conditional = parent.conditional;
    for (condition, format) in child.conditional {
        let inherited = match conditional.remove(&condition) {
            Some(parent) => TableStyleConditionalFormat::inherit(parent, format),
            None => format,
        };
        conditional.insert(condition, inherited);
    }
    TableStyleDisplay {
        conditional,
        row_band_size: child.row_band_size.or(parent.row_band_size),
        col_band_size: child.col_band_size.or(parent.col_band_size),
        fill: child.fill.or(parent.fill),
        first_row,
        band1_fill: child.band1_fill.or(parent.band1_fill),
        band2_fill: child.band2_fill.or(parent.band2_fill),
        borders: match (parent.borders, child.borders) {
            (Some(parent), Some(child)) => Some(merge_table_borders(parent, child)),
            (parent, child) => child.or(parent),
        },
        cell_margins: match (parent.cell_margins, child.cell_margins) {
            (Some(parent), Some(child)) => Some(merge_table_margins(parent, child)),
            (parent, child) => child.or(parent),
        },
    }
}

fn apply_to_active_run_properties(
    styles: &mut StylesPart,
    current_style: &mut Option<StyleDefinition>,
    state: &StyleParseState,
    apply: impl FnOnce(&mut RunProperties),
) {
    if state.in_style_run_properties {
        if let Some(properties) = current_style
            .as_mut()
            .and_then(|style| style.run_properties.as_mut())
        {
            apply(properties);
        }
    } else if state.in_doc_default_run_properties {
        if let Some(properties) = styles.doc_default_run_properties.as_mut() {
            apply(properties);
        }
    }
}

/// Minimal representation of a paragraph style used by the renderer.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StyleDefinition {
    pub style_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub based_on: Option<String>,
    /// `w:next`: the style Word gives the paragraph created by Enter at the
    /// end of a paragraph in this style (a heading is followed by Normal).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_style: Option<String>,
    /// The other half of a Word linked paragraph/character style pair.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linked_style: Option<String>,
    /// `w:semiHidden`; hidden from galleries while remaining addressable by id.
    #[serde(default)]
    pub semi_hidden: bool,
    #[serde(default)]
    pub is_default: bool,
    pub numbering: Option<Numbering>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paragraph_properties: Option<ParagraphProperties>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_properties: Option<RunProperties>,
}

fn handle_table_cell_margin_style_start(
    element: &BytesStart,
    current: &mut Option<(String, TableCellMarginStyleDefinition)>,
    in_cell_margins: &mut bool,
) {
    match element.name().as_ref() {
        b"w:style" => {
            *in_cell_margins = false;
            *current = if attr_value(element, b"w:type").as_deref() == Some("table") {
                attr_value(element, b"w:styleId")
                    .map(|style_id| (style_id, TableCellMarginStyleDefinition::default()))
            } else {
                None
            };
        }
        b"w:basedOn" => {
            if let Some((_, definition)) = current.as_mut() {
                definition.based_on = attr_value(element, b"w:val");
            }
        }
        b"w:tblCellMar" if current.is_some() => *in_cell_margins = true,
        b"w:top" | b"w:bottom" | b"w:left" | b"w:right" | b"w:start" | b"w:end"
            if *in_cell_margins =>
        {
            let width_type = attr_value(element, b"w:type").unwrap_or_else(|| "dxa".to_string());
            if width_type != "dxa" {
                return;
            }
            let Some(points) = attr_value(element, b"w:w")
                .and_then(|value| value.parse::<f32>().ok())
                .map(|twips| twips / 20.0)
            else {
                return;
            };
            let Some((_, definition)) = current.as_mut() else {
                return;
            };
            let margins = definition
                .margins
                .get_or_insert_with(TableCellMargins::default);
            match element.name().as_ref() {
                b"w:top" => margins.top = Some(points),
                b"w:bottom" => margins.bottom = Some(points),
                b"w:left" => margins.left = Some(points),
                b"w:right" => margins.right = Some(points),
                b"w:start" => margins.start = Some(points),
                b"w:end" => margins.end = Some(points),
                _ => {}
            }
        }
        _ => {}
    }
}

fn merge_table_cell_margins(parent: TableCellMargins, child: TableCellMargins) -> TableCellMargins {
    TableCellMargins {
        top: child.top.or(parent.top),
        bottom: child.bottom.or(parent.bottom),
        left: child.left.or(parent.left),
        right: child.right.or(parent.right),
        start: child.start.or(parent.start),
        end: child.end.or(parent.end),
    }
}

fn attr_value(element: &BytesStart, key: &[u8]) -> Option<String> {
    for attr in element.attributes().flatten() {
        if attr.key.as_ref() == key {
            return Some(String::from_utf8_lossy(&attr.value).to_string());
        }
    }
    None
}

fn normalize_style_lookup_key(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter_map(|ch| {
            if ch.is_whitespace() || ch == '_' || ch == '-' {
                None
            } else {
                Some(ch.to_ascii_lowercase())
            }
        })
        .collect()
}

fn parse_on_off_value(raw: &str) -> bool {
    !matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "off"
    )
}

/// Parses `ST_Underline` as an effective on/off value.
///
/// Unlike the generic OOXML on/off type, underline uses the dedicated token
/// `none` to disable the decoration. Other line-style tokens (`single`,
/// `double`, `wave`, and so on) all mean that an underline is present.
fn parse_underline_value(raw: &str) -> bool {
    !raw.trim().eq_ignore_ascii_case("none") && parse_on_off_value(raw)
}

fn parse_on_off_attr(element: &BytesStart, key: &[u8]) -> bool {
    attr_value(element, key)
        .as_deref()
        .map(parse_on_off_value)
        .unwrap_or(true)
}

fn parse_font_size(element: &BytesStart) -> Option<f32> {
    for attr in element.attributes().flatten() {
        if attr.key.as_ref() == b"w:val" {
            if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                return Some(value / 2.0);
            }
        }
    }
    None
}

fn parse_indent_definition(element: &BytesStart) -> Option<Indent> {
    let mut indent = Indent::default();
    let mut updated = false;

    for attr in element.attributes().flatten() {
        match attr.key.as_ref() {
            b"w:left" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                    indent.left = Some(value / 20.0);
                    updated = true;
                }
            }
            b"w:right" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                    indent.right = Some(value / 20.0);
                    updated = true;
                }
            }
            b"w:start" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                    indent.start = Some(value / 20.0);
                    updated = true;
                }
            }
            b"w:end" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                    indent.end = Some(value / 20.0);
                    updated = true;
                }
            }
            b"w:firstLine" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                    indent.first_line = Some(value / 20.0);
                    updated = true;
                }
            }
            b"w:hanging" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                    indent.hanging = Some(value / 20.0);
                    updated = true;
                }
            }
            b"w:leftChars" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<i32>() {
                    indent.left_chars = Some(value);
                    updated = true;
                }
            }
            b"w:rightChars" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<i32>() {
                    indent.right_chars = Some(value);
                    updated = true;
                }
            }
            b"w:startChars" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<i32>() {
                    indent.start_chars = Some(value);
                    updated = true;
                }
            }
            b"w:endChars" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<i32>() {
                    indent.end_chars = Some(value);
                    updated = true;
                }
            }
            b"w:firstLineChars" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<i32>() {
                    indent.first_line_chars = Some(value);
                    updated = true;
                }
            }
            b"w:hangingChars" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<i32>() {
                    indent.hanging_chars = Some(value);
                    updated = true;
                }
            }
            _ => {}
        }
    }

    updated.then_some(indent)
}

fn parse_spacing_definition(element: &BytesStart) -> Option<Spacing> {
    let mut spacing = Spacing::default();
    let mut line_raw: Option<f32> = None;
    let mut updated = false;
    let (mut before_auto, mut after_auto) = (false, false);

    for attr in element.attributes().flatten() {
        match attr.key.as_ref() {
            b"w:before" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                    spacing.before = Some(value / 20.0);
                    updated = true;
                }
            }
            b"w:after" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                    spacing.after = Some(value / 20.0);
                    updated = true;
                }
            }
            b"w:beforeLines" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                    spacing.before_lines = Some(value / 100.0);
                    updated = true;
                }
            }
            b"w:afterLines" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                    spacing.after_lines = Some(value / 100.0);
                    updated = true;
                }
            }
            b"w:line" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                    line_raw = Some(value);
                }
            }
            b"w:lineRule" => {
                let rule = String::from_utf8_lossy(&attr.value).to_string();
                spacing.line_rule = Some(match rule.as_str() {
                    "exactly" => "exact".to_string(),
                    other => other.to_string(),
                });
                updated = true;
            }
            b"w:beforeAutospacing" => {
                before_auto = crate::document::ooxml_on(&attr.value);
            }
            b"w:afterAutospacing" => {
                after_auto = crate::document::ooxml_on(&attr.value);
            }
            _ => {}
        }
    }
    if before_auto || after_auto {
        spacing.apply_autospacing(before_auto, after_auto);
        updated = true;
    }

    if let Some(raw) = line_raw {
        let rule = spacing.line_rule.as_deref().unwrap_or("auto");
        spacing.line = Some(match rule {
            "atLeast" | "exact" | "exactly" => raw / 20.0,
            _ => raw / 240.0,
        });
        if spacing.line_rule.is_none() {
            spacing.line_rule = Some("auto".to_string());
        }
        updated = true;
    }

    updated.then_some(spacing)
}

fn parse_kerning_points(element: &BytesStart) -> Option<f32> {
    for attr in element.attributes().flatten() {
        if attr.key.as_ref() == b"w:val" {
            if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                let points = value / 2.0;
                if points.is_finite() && points >= 0.0 {
                    return Some(points);
                }
            }
        }
    }
    None
}

fn parse_text_scale_percent(element: &BytesStart) -> Option<f32> {
    for attr in element.attributes().flatten() {
        if attr.key.as_ref() == b"w:val" {
            if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                if value.is_finite() && value > 0.0 {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn parse_fit_text(element: &BytesStart) -> Option<RunFitText> {
    let mut val = None;
    let mut id = None;

    for attr in element.attributes().flatten() {
        match attr.key.as_ref() {
            b"w:val" => {
                val = String::from_utf8_lossy(&attr.value).parse::<u32>().ok();
            }
            b"w:id" => {
                id = String::from_utf8_lossy(&attr.value).parse::<i32>().ok();
            }
            _ => {}
        }
    }

    val.map(|val| RunFitText { val, id })
}

fn apply_run_fonts(props: &mut RunProperties, element: &BytesStart) {
    props.authored_east_asia_font_hint = Some(element.attributes().flatten().any(|attr| {
        attr.key.as_ref() == b"w:hint"
            && FontHint::from_xml_hint(&String::from_utf8_lossy(&attr.value))
                == Some(FontHint::EastAsia)
    }));
    props.authored_east_asia_font_name = Some(
        element
            .attributes()
            .flatten()
            .any(|attr| matches!(attr.key.as_ref(), b"w:eastAsia" | b"w:eastAsiaTheme")),
    );
    let fonts = props.fonts.get_or_insert_with(RunFonts::default);
    for attr in element.attributes().flatten() {
        let key = attr.key.as_ref();
        let value = String::from_utf8_lossy(&attr.value).to_string();
        match key {
            b"w:ascii" => {
                fonts.ascii = Some(Rc::from(value));
            }
            b"w:hAnsi" => {
                fonts.h_ansi = Some(Rc::from(value));
            }
            b"w:eastAsia" => {
                fonts.east_asia = Some(Rc::from(value));
            }
            b"w:cs" => {
                fonts.complex = Some(Rc::from(value));
            }
            b"w:asciiTheme" => {
                fonts.ascii_theme = Some(Rc::from(value));
            }
            b"w:hAnsiTheme" => {
                fonts.h_ansi_theme = Some(Rc::from(value));
            }
            b"w:eastAsiaTheme" => {
                fonts.east_asia_theme = Some(Rc::from(value));
            }
            b"w:cstheme" => {
                fonts.complex_theme = Some(Rc::from(value));
            }
            b"w:hint" => {
                fonts.hint = FontHint::from_xml_hint(&value);
            }
            _ => {}
        }
    }
    fonts.drop_names_shadowed_by_themes();
    if props.font_family.is_none() {
        if let Some(family) = fonts.preferred_latin() {
            props.font_family = Some(Rc::from(family));
        }
    }
}

/// Merges `source` run properties into `target`, with `source` values taking
/// precedence over existing `target` values when both are present.
pub fn merge_run_properties_override(target: &mut RunProperties, source: &RunProperties) {
    if source.bold.is_some() {
        target.bold = source.bold;
    }
    if source.italic.is_some() {
        target.italic = source.italic;
    }
    if source.bold_complex.is_some() {
        target.bold_complex = source.bold_complex;
    }
    if source.italic_complex.is_some() {
        target.italic_complex = source.italic_complex;
    }
    if source.underline.is_some() {
        target.underline = source.underline;
        // The kind/color travel with the `w:u` declaration that set on/off:
        // a style that re-declares underline replaces the inherited flavor.
        target.underline_kind = source.underline_kind.clone();
        target.underline_color = source.underline_color.clone();
    }
    if source.strike.is_some() {
        target.strike = source.strike;
    }
    if source.double_strike.is_some() {
        target.double_strike = source.double_strike;
    }
    if source.caps.is_some() {
        target.caps = source.caps;
    }
    if source.small_caps.is_some() {
        target.small_caps = source.small_caps;
    }
    if source.vanish.is_some() {
        target.vanish = source.vanish;
    }
    if source.spec_vanish.is_some() {
        target.spec_vanish = source.spec_vanish;
    }
    if source.shadow.is_some() {
        target.shadow = source.shadow;
    }
    if source.outline.is_some() {
        target.outline = source.outline;
    }
    if source.emboss.is_some() {
        target.emboss = source.emboss;
    }
    if source.imprint.is_some() {
        target.imprint = source.imprint;
    }
    if source.emphasis.is_some() {
        target.emphasis = source.emphasis.clone();
    }
    if source.effect.is_some() {
        target.effect = source.effect.clone();
    }
    if source.baseline.is_some() {
        target.baseline = source.baseline.clone();
    }
    if source.text_position.is_some() {
        target.text_position = source.text_position;
    }
    if source.char_spacing.is_some() {
        target.char_spacing = source.char_spacing;
    }
    if source.kerning.is_some() {
        target.kerning = source.kerning;
    }
    if source.text_scale.is_some() {
        target.text_scale = source.text_scale;
    }
    if source.fit_text.is_some() {
        target.fit_text = source.fit_text.clone();
    }
    if source.font_size_complex.is_some() {
        target.font_size_complex = source.font_size_complex;
    }
    if source.font_size.is_some() {
        target.font_size = source.font_size;
    }
    if source.font_family.is_some() {
        target.font_family = source.font_family.clone();
    }
    match (&mut target.fonts, source.fonts.as_ref()) {
        (Some(target_fonts), Some(source_fonts)) => {
            target_fonts.merge_from(source_fonts);
        }
        (None, Some(source_fonts)) => {
            target.fonts = Some(source_fonts.clone());
        }
        _ => {}
    }
    if source.language.is_some() {
        target.language = source.language.clone();
    }
    if source.east_asia_language.is_some() {
        target.east_asia_language = source.east_asia_language.clone();
    }
    if source.authored_east_asia_font_hint.is_some() {
        target.authored_east_asia_font_hint = source.authored_east_asia_font_hint;
    }
    if source.authored_east_asia_font_name.is_some() {
        target.authored_east_asia_font_name = source.authored_east_asia_font_name;
    }
    if source.color.is_some() {
        target.color = source.color.clone();
    }
    if source.highlight.is_some() {
        target.highlight = source.highlight.clone();
    }
    if source.shading.is_some() {
        target.shading = source.shading.clone();
    }
    if source.border.is_some() {
        target.border = source.border.clone();
    }
    if source.ruby.is_some() {
        target.ruby = source.ruby.clone();
    }
    // Word 2010 typography and text effects (`w14:ligatures`, `w14:glow`, …)
    // inherit setting by setting, as the other run properties do.
    if let Some(source_effects) = source.text_effects.as_deref() {
        let effects = target.text_effects.get_or_insert_with(Default::default);
        macro_rules! inherit {
            ($($field:ident),*) => {
                $(if source_effects.$field.is_some() {
                    effects.$field = source_effects.$field.clone();
                })*
            };
        }
        inherit!(
            ligatures,
            number_form,
            number_spacing,
            stylistic_set,
            contextual_alternates,
            glow_color,
            glow_radius,
            glow_alpha,
            outline_color,
            outline_width,
            enclosure
        );
    }
}

fn merge_indent_override(target: &mut Indent, source: &Indent) {
    if source.left.is_some() {
        target.left = source.left;
    }
    if source.right.is_some() {
        target.right = source.right;
    }
    if source.start.is_some() {
        target.start = source.start;
    }
    if source.end.is_some() {
        target.end = source.end;
    }
    if source.first_line.is_some() {
        target.first_line = source.first_line;
    }
    if source.hanging.is_some() {
        target.hanging = source.hanging;
    }
    if source.left_chars.is_some() {
        target.left_chars = source.left_chars;
    }
    if source.right_chars.is_some() {
        target.right_chars = source.right_chars;
    }
    if source.start_chars.is_some() {
        target.start_chars = source.start_chars;
    }
    if source.end_chars.is_some() {
        target.end_chars = source.end_chars;
    }
    if source.first_line_chars.is_some() {
        target.first_line_chars = source.first_line_chars;
    }
    if source.hanging_chars.is_some() {
        target.hanging_chars = source.hanging_chars;
    }
}

fn merge_spacing_override(target: &mut Spacing, source: &Spacing) {
    if source.before.is_some() {
        target.before = source.before;
        target.before_auto = source.before_auto;
    }
    if source.after.is_some() {
        target.after = source.after;
        target.after_auto = source.after_auto;
    }
    if source.before_lines.is_some() {
        target.before_lines = source.before_lines;
        target.before_auto = false;
    }
    if source.after_lines.is_some() {
        target.after_lines = source.after_lines;
        target.after_auto = false;
    }
    if source.line.is_some() {
        target.line = source.line;
    }
    if source.line_rule.is_some() {
        target.line_rule = source.line_rule.clone();
    }
}

fn merge_east_asian_settings_override(
    target: &mut EastAsianLineBreakingSettings,
    source: &EastAsianLineBreakingSettings,
) {
    target.use_east_asian_layout |= source.use_east_asian_layout;
    if source.kinsoku.is_some() {
        target.kinsoku = source.kinsoku;
    }
    if source.word_wrap.is_some() {
        target.word_wrap = source.word_wrap;
    }
    if source.overflow_punctuation.is_some() {
        target.overflow_punctuation = source.overflow_punctuation;
    }
    if source.top_line_punctuation.is_some() {
        target.top_line_punctuation = source.top_line_punctuation;
    }
    target.balance_single_byte_double_byte_width |= source.balance_single_byte_double_byte_width;
    if source.auto_space_de.is_some() {
        target.auto_space_de = source.auto_space_de;
    }
    if source.auto_space_dn.is_some() {
        target.auto_space_dn = source.auto_space_dn;
    }
    if source.character_spacing_control.is_some() {
        target.character_spacing_control = source.character_spacing_control.clone();
    }
    if !source.no_line_break_before.is_empty() {
        target.no_line_break_before = source.no_line_break_before.clone();
    }
    if !source.no_line_break_after.is_empty() {
        target.no_line_break_after = source.no_line_break_after.clone();
    }
}

/// Merges `source` paragraph properties into `target`, with `source` values
/// taking precedence over existing `target` values when both are present.
pub fn merge_paragraph_properties_override(
    target: &mut ParagraphProperties,
    source: &ParagraphProperties,
) {
    if source.mark_run_properties.is_some() {
        target.mark_run_properties = source.mark_run_properties.clone();
    }
    if source.mark_revision.is_some() {
        target.mark_revision = source.mark_revision.clone();
    }
    if source.alignment.is_some() {
        target.alignment = source.alignment.clone();
    }
    if source.bidi.is_some() {
        target.bidi = source.bidi;
    }
    match (&mut target.spacing, source.spacing.as_ref()) {
        (Some(target_spacing), Some(source_spacing)) => {
            merge_spacing_override(target_spacing, source_spacing);
        }
        (None, Some(source_spacing)) => {
            target.spacing = Some(source_spacing.clone());
        }
        _ => {}
    }
    match (&mut target.indent, source.indent.as_ref()) {
        (Some(target_indent), Some(source_indent)) => {
            merge_indent_override(target_indent, source_indent);
        }
        (None, Some(source_indent)) => {
            target.indent = Some(source_indent.clone());
        }
        _ => {}
    }
    if !source.tabs.is_empty() {
        target.tabs = source.tabs.clone();
    }
    if source.adjust_right_indent.is_some() {
        target.adjust_right_indent = source.adjust_right_indent;
    }
    if source.snap_to_grid.is_some() {
        target.snap_to_grid = source.snap_to_grid;
    }
    if source.widow_control.is_some() {
        target.widow_control = source.widow_control;
    }
    if source.suppress_auto_hyphens.is_some() {
        target.suppress_auto_hyphens = source.suppress_auto_hyphens;
    }
    if source.page_break_before.is_some() {
        target.page_break_before = source.page_break_before;
    }
    if source.frame.is_some() {
        target.frame = source.frame;
    }
    if source.suppress_line_numbers.is_some() {
        target.suppress_line_numbers = source.suppress_line_numbers;
    }
    if source.keep_lines.is_some() {
        target.keep_lines = source.keep_lines;
    }
    if source.keep_next.is_some() {
        target.keep_next = source.keep_next;
    }
    if source.text_alignment.is_some() {
        target.text_alignment = source.text_alignment.clone();
    }
    if source.mirror_indents.is_some() {
        target.mirror_indents = source.mirror_indents;
    }
    if source.outline_level.is_some() {
        target.outline_level = source.outline_level;
    }
    if source.shading.is_some() {
        target.shading = source.shading.clone();
    }
    if source.borders.is_some() {
        target.borders = source.borders.clone();
    }
    if source.numbering.is_some() {
        target.numbering = source.numbering.clone();
    }
    if source.contextual_spacing.is_some() {
        target.contextual_spacing = source.contextual_spacing;
    }
    match (
        &mut target.east_asian_settings,
        source.east_asian_settings.as_ref(),
    ) {
        (Some(target_settings), Some(source_settings)) => {
            merge_east_asian_settings_override(target_settings, source_settings);
        }
        (None, Some(source_settings)) => {
            target.east_asian_settings = Some(source_settings.clone());
        }
        _ => {}
    }
}

fn ensure_numbering(style: &mut StyleDefinition) {
    if style.numbering.is_none() {
        style.numbering = Some(Numbering {
            id: None,
            level: None,
        });
    }
}

fn parse_tab_definition(element: &BytesStart) -> Option<TabStop> {
    let mut stop = TabStop::default();
    for attr in element.attributes().flatten() {
        match attr.key.as_ref() {
            b"w:val" => {
                stop.kind = Some(String::from_utf8_lossy(&attr.value).to_string());
            }
            b"w:pos" => {
                if let Ok(value) = String::from_utf8_lossy(&attr.value)
                    .to_string()
                    .parse::<f32>()
                {
                    stop.position = Some(value / 20.0);
                }
            }
            b"w:leader" => {
                stop.leader = Some(String::from_utf8_lossy(&attr.value).to_string());
            }
            _ => {}
        }
    }
    if stop.kind.is_none() && stop.position.is_none() && stop.leader.is_none() {
        None
    } else {
        Some(stop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `w:szCs` sizes complex-script text only: Word for Mac draws Latin and
    /// Japanese text in a style that declares `w:szCs` alone at the size it
    /// inherits, so the style must not take it as its `w:sz`.
    #[test]
    fn complex_script_size_alone_leaves_the_size_inherited() {
        let xml = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr><w:szCs w:val="40"/></w:rPr></w:rPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:rPr><w:sz w:val="20"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Cs"><w:basedOn w:val="Normal"/><w:rPr><w:szCs w:val="40"/></w:rPr></w:style>
</w:styles>"#;
        let styles = StylesPart::from_xml(xml).expect("styles");
        let defaults = styles.doc_default_run_properties().expect("defaults");
        assert_eq!(defaults.font_size, None);
        assert_eq!(defaults.font_size_complex, Some(20.0));
        let resolved = styles.resolve_run_properties("Cs").expect("style");
        assert_eq!(resolved.font_size, Some(10.0));
        assert_eq!(resolved.font_size_complex, Some(20.0));
    }

    #[test]
    fn auto_spacing_is_fourteen_points_whatever_the_authored_value() {
        // Word's Normal (Web) after an HTML import: `w:before="100"` beside
        // `w:beforeAutospacing="1"` is drawn 14pt, as the paragraph dialog's
        // "auto" says.
        let xml = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:style w:type="paragraph" w:styleId="Web"><w:pPr><w:spacing w:before="100" w:beforeAutospacing="1" w:after="100" w:afterAutospacing="0"/></w:pPr></w:style>
</w:styles>"#;
        let styles = StylesPart::from_xml(xml).unwrap();
        let spacing = styles
            .resolve_paragraph_properties("Web")
            .unwrap()
            .spacing
            .unwrap();
        assert_eq!(spacing.before, Some(14.0));
        assert_eq!(spacing.after, Some(5.0));
    }

    #[test]
    fn style_run_properties_read_character_spacing() {
        let xml = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:style w:type="paragraph" w:styleId="Title"><w:pPr><w:spacing w:after="300"/></w:pPr>
<w:rPr><w:spacing w:val="5"/></w:rPr></w:style></w:styles>"#;
        let styles = StylesPart::from_xml(xml).unwrap();
        assert_eq!(
            styles.resolve_run_properties("Title").unwrap().char_spacing,
            Some(0.25)
        );
    }

    #[test]
    fn theme_attribute_shadows_the_font_name_of_the_same_element() {
        let xml = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:style w:type="paragraph" w:styleId="Title"><w:rPr>
<w:rFonts w:asciiTheme="majorHAnsi" w:hAnsiTheme="majorHAnsi" w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="MS Mincho"/>
</w:rPr></w:style></w:styles>"#;
        let styles = StylesPart::from_xml(xml).unwrap();
        let fonts = styles
            .resolve_run_properties("Title")
            .unwrap()
            .fonts
            .unwrap();
        assert_eq!(fonts.ascii, None);
        assert_eq!(fonts.h_ansi, None);
        assert_eq!(fonts.ascii_theme.as_deref(), Some("majorHAnsi"));
        assert_eq!(fonts.east_asia.as_deref(), Some("MS Mincho"));
    }

    #[test]
    fn paragraph_style_spacing_uses_hundredths_of_a_line() {
        let xml = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:style w:type="paragraph" w:styleId="lineSpacing">
            <w:pPr><w:spacing w:beforeLines="50" w:afterLines="150" w:line="360" w:lineRule="auto"/></w:pPr>
          </w:style></w:styles>"#;
        let part = StylesPart::from_xml(xml).unwrap();
        let spacing = part
            .get_style("lineSpacing")
            .unwrap()
            .paragraph_properties
            .as_ref()
            .unwrap()
            .spacing
            .as_ref()
            .unwrap();
        assert_eq!(spacing.before_lines, Some(0.5));
        assert_eq!(spacing.after_lines, Some(1.5));
        assert_eq!(spacing.line, Some(1.5));
    }

    #[test]
    fn paragraph_override_exposes_direct_numbering_to_caret_consumers() {
        let mut target = ParagraphProperties::default();
        let source = ParagraphProperties {
            numbering: Some(Numbering {
                id: Some("1000".to_string()),
                level: Some(7),
            }),
            ..ParagraphProperties::default()
        };
        merge_paragraph_properties_override(&mut target, &source);
        let numbering = target.numbering.expect("numbering copied");
        assert_eq!(numbering.id.as_deref(), Some("1000"));
        assert_eq!(numbering.level, Some(7));
    }

    #[test]
    fn paragraph_direction_is_inherited_and_can_be_explicitly_overridden() {
        let mut target = ParagraphProperties {
            bidi: Some(true),
            ..ParagraphProperties::default()
        };
        let source = ParagraphProperties {
            bidi: Some(false),
            ..ParagraphProperties::default()
        };
        merge_paragraph_properties_override(&mut target, &source);
        assert_eq!(target.bidi, Some(false));
    }

    #[test]
    fn parses_paragraph_style_direction() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="rtlStyle">
    <w:pPr><w:bidi/></w:pPr>
  </w:style>
</w:styles>
"#;
        let part = StylesPart::from_xml(xml).expect("parse styles");
        let bidi = part
            .get_style("rtlStyle")
            .and_then(|style| style.paragraph_properties.as_ref())
            .and_then(|props| props.bidi);
        assert_eq!(bidi, Some(true));
    }

    #[test]
    fn parses_logical_indents_from_paragraph_styles() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="logicalIndent">
    <w:pPr><w:ind w:start="720" w:end="360" w:startChars="200" w:endChars="100"/></w:pPr>
  </w:style>
</w:styles>
"#;
        let part = StylesPart::from_xml(xml).expect("parse styles");
        let indent = part
            .get_style("logicalIndent")
            .and_then(|style| style.paragraph_properties.as_ref())
            .and_then(|props| props.indent.as_ref())
            .expect("logical indent");

        assert_eq!(indent.physical_left(false), Some(36.0));
        assert_eq!(indent.physical_right(false), Some(18.0));
        assert_eq!(indent.physical_left(true), Some(18.0));
        assert_eq!(indent.physical_right(true), Some(36.0));
        assert_eq!(indent.physical_left_chars(false), Some(200));
        assert_eq!(indent.physical_right_chars(true), Some(200));
    }

    #[test]
    fn parses_doc_default_paragraph_on_off_properties() {
        // Word for Mac 16.114 leaves a single line of a paragraph at a page
        // top when `w:pPrDefault` turns widow control off, so these defaults
        // must reach the cascade like any style's.
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:pPrDefault>
      <w:pPr>
        <w:widowControl w:val="0"/>
        <w:keepNext/>
        <w:keepLines/>
        <w:pageBreakBefore w:val="0"/>
        <w:snapToGrid w:val="0"/>
        <w:suppressAutoHyphens/>
        <w:suppressLineNumbers/>
        <w:contextualSpacing/>
      </w:pPr>
    </w:pPrDefault>
  </w:docDefaults>
</w:styles>
"#;
        let part = StylesPart::from_xml(xml).expect("parse styles");
        let defaults = part
            .doc_default_paragraph_properties()
            .expect("pPrDefault parsed");
        assert_eq!(defaults.widow_control, Some(false));
        assert_eq!(defaults.keep_next, Some(true));
        assert_eq!(defaults.keep_lines, Some(true));
        assert_eq!(defaults.page_break_before, Some(false));
        assert_eq!(defaults.snap_to_grid, Some(false));
        assert_eq!(defaults.suppress_auto_hyphens, Some(true));
        assert_eq!(defaults.suppress_line_numbers, Some(true));
        assert_eq!(defaults.contextual_spacing, Some(true));
    }

    #[test]
    fn parses_doc_default_paragraph_properties() {
        // `w:docDefaults/w:pPrDefault/w:pPr` carries the document-wide
        // paragraph defaults (common in Google Docs / LibreOffice exports).
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault><w:rPr><w:sz w:val="21"/></w:rPr></w:rPrDefault>
    <w:pPrDefault>
      <w:pPr>
        <w:ind w:left="420" w:firstLineChars="100"/>
        <w:spacing w:after="160"/>
        <w:jc w:val="both"/>
      </w:pPr>
    </w:pPrDefault>
  </w:docDefaults>
  <w:style w:type="paragraph" w:styleId="plain">
    <w:pPr><w:jc w:val="center"/></w:pPr>
  </w:style>
</w:styles>
"#;
        let part = StylesPart::from_xml(xml).expect("parse styles");
        let defaults = part
            .doc_default_paragraph_properties()
            .expect("pPrDefault parsed");
        let indent = defaults.indent.as_ref().expect("default indent");
        assert_eq!(indent.left, Some(21.0));
        assert_eq!(indent.first_line_chars, Some(100));
        let spacing = defaults.spacing.as_ref().expect("default spacing");
        assert_eq!(spacing.after, Some(8.0), "160 twips = 8pt");
        assert_eq!(defaults.alignment.as_deref(), Some("both"));

        // The defaults must not leak into named styles (and vice versa).
        let style = part
            .get_style("plain")
            .and_then(|style| style.paragraph_properties.as_ref())
            .expect("style props");
        assert_eq!(style.alignment.as_deref(), Some("center"));
        assert!(style.indent.is_none());
        assert!(part
            .doc_default_run_properties()
            .and_then(|props| props.font_size)
            .is_some());
    }

    #[test]
    fn parses_paragraph_style_auto_space_flags() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="jpStyle">
    <w:pPr>
      <w:autoSpaceDE w:val="0"/>
      <w:autoSpaceDN/>
    </w:pPr>
  </w:style>
</w:styles>
"#;

        let part = StylesPart::from_xml(xml).expect("parse styles");
        let style = part.get_style("jpStyle").expect("style");
        let settings = style
            .paragraph_properties
            .as_ref()
            .and_then(|props| props.east_asian_settings.as_ref())
            .expect("east asian settings");
        assert_eq!(settings.auto_space_de, Some(false));
        assert_eq!(settings.auto_space_dn, Some(true));
    }

    #[test]
    fn parses_paragraph_style_line_break_typography_flags() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="jpStyle">
    <w:pPr>
      <w:kinsoku w:val="0"/>
      <w:wordWrap w:val="0"/>
      <w:overflowPunct/>
      <w:topLinePunct w:val="1"/>
    </w:pPr>
  </w:style>
</w:styles>
"#;

        let part = StylesPart::from_xml(xml).expect("parse styles");
        let style = part.get_style("jpStyle").expect("style");
        let settings = style
            .paragraph_properties
            .as_ref()
            .and_then(|props| props.east_asian_settings.as_ref())
            .expect("east asian settings");
        assert_eq!(settings.kinsoku, Some(false));
        assert_eq!(settings.word_wrap, Some(false));
        assert_eq!(settings.overflow_punctuation, Some(true));
        assert_eq!(settings.top_line_punctuation, Some(true));
    }

    #[test]
    fn parses_paragraph_style_shading_and_borders() {
        // A style can carry paragraph shading (`w:shd`) and borders (`w:pBdr`),
        // mirroring direct `w:pPr` formatting. ECMA-376 §17.3.5 / §17.3.1.24.
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="boxed">
    <w:pPr>
      <w:shd w:val="clear" w:color="auto" w:fill="FFFF00"/>
      <w:pBdr>
        <w:top w:val="single" w:sz="8" w:space="4" w:color="FF0000"/>
        <w:bottom w:val="dashed" w:sz="12" w:color="auto"/>
      </w:pBdr>
    </w:pPr>
  </w:style>
</w:styles>
"#;

        let part = StylesPart::from_xml(xml).expect("parse styles");
        let props = part
            .get_style("boxed")
            .and_then(|style| style.paragraph_properties.as_ref())
            .expect("paragraph properties");

        let shading = props.shading.as_ref().expect("shading");
        assert_eq!(shading.fill.as_deref(), Some("#FFFF00"));

        let borders = props.borders.as_ref().expect("borders");
        let top = borders.top.as_ref().expect("top edge");
        assert_eq!(top.val.as_deref(), Some("single"));
        assert_eq!(top.size_eighths, Some(8));
        assert_eq!(top.space, Some(4));
        assert_eq!(top.color.as_deref(), Some("#FF0000"));
        let bottom = borders.bottom.as_ref().expect("bottom edge");
        assert_eq!(bottom.val.as_deref(), Some("dashed"));
        // `w:color="auto"` normalizes to None.
        assert_eq!(bottom.color, None);
        // Unspecified edges stay absent.
        assert!(borders.left.is_none());
    }

    #[test]
    fn resolves_style_frame_properties_but_not_drop_caps() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="framed">
    <w:pPr><w:framePr w:w="2000" w:wrap="around" w:hAnchor="page" w:x="500"/></w:pPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="derived"><w:basedOn w:val="framed"/></w:style>
  <w:style w:type="paragraph" w:styleId="cap">
    <w:pPr><w:framePr w:dropCap="drop" w:lines="3"/></w:pPr>
  </w:style>
</w:styles>
"#;
        let part = StylesPart::from_xml(xml).expect("parse styles");
        let frame = part
            .resolve_paragraph_properties("derived")
            .and_then(|props| props.frame)
            .expect("inherited frame");
        assert_eq!(frame.width_twips, Some(2000));
        assert_eq!(frame.x_twips, Some(500));
        assert!(part
            .resolve_paragraph_properties("cap")
            .is_none_or(|props| props.frame.is_none()));
    }

    #[test]
    fn adjust_right_indent_document_defaults_preserve_false_and_style_override() {
        let part = StylesPart::from_xml(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:docDefaults><w:pPrDefault><w:pPr><w:adjustRightInd w:val="off"/></w:pPr></w:pPrDefault></w:docDefaults>
            <w:style w:type="paragraph" w:styleId="on"><w:pPr><w:adjustRightInd/></w:pPr></w:style>
        </w:styles>"#).unwrap();
        let mut effective = part.doc_default_paragraph_properties().unwrap().clone();
        assert_eq!(effective.adjust_right_indent, Some(false));
        merge_paragraph_properties_override(&mut effective, &ParagraphProperties::default());
        assert_eq!(effective.adjust_right_indent, Some(false));
        merge_paragraph_properties_override(
            &mut effective,
            &part.resolve_paragraph_properties("on").unwrap(),
        );
        assert_eq!(effective.adjust_right_indent, Some(true));
    }

    #[test]
    fn adjust_right_indent_inherits_and_direct_false_overrides_style_true() {
        let part = StylesPart::from_xml(r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:style w:type="paragraph" w:styleId="base"><w:pPr><w:adjustRightInd/></w:pPr></w:style>
            <w:style w:type="paragraph" w:styleId="child"><w:basedOn w:val="base"/></w:style>
            <w:style w:type="paragraph" w:styleId="off"><w:basedOn w:val="child"/><w:pPr><w:adjustRightInd w:val="0"/></w:pPr></w:style>
        </w:styles>"#).unwrap();
        let mut child = part.resolve_paragraph_properties("child").unwrap();
        assert_eq!(
            serde_json::to_value(&child).unwrap()["adjust_right_indent"],
            true
        );
        let off = part.resolve_paragraph_properties("off").unwrap();
        assert_eq!(
            serde_json::to_value(&off).unwrap()["adjust_right_indent"],
            false
        );
        merge_paragraph_properties_override(&mut child, &off);
        assert_eq!(
            serde_json::to_value(&child).unwrap()["adjust_right_indent"],
            false
        );
    }

    #[test]
    fn resolves_paragraph_shading_and_borders_across_based_on_chain() {
        // The base style defines shading; the derived style defines borders.
        // Resolution following `basedOn` must surface both.
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="base">
    <w:pPr>
      <w:shd w:val="clear" w:color="auto" w:fill="00FF00"/>
    </w:pPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="derived">
    <w:basedOn w:val="base"/>
    <w:pPr>
      <w:pBdr>
        <w:top w:val="single" w:sz="4" w:color="000000"/>
      </w:pBdr>
    </w:pPr>
  </w:style>
</w:styles>
"#;

        let part = StylesPart::from_xml(xml).expect("parse styles");
        let resolved = part
            .resolve_paragraph_properties("derived")
            .expect("resolved props");
        assert_eq!(
            resolved.shading.as_ref().and_then(|s| s.fill.as_deref()),
            Some("#00FF00")
        );
        assert!(resolved
            .borders
            .as_ref()
            .and_then(|b| b.top.as_ref())
            .is_some());
    }

    #[test]
    fn fit_text_preserves_signed_group_ids_in_defaults_and_styles() {
        let xml = r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
          <w:docDefaults><w:rPrDefault><w:rPr><w:fitText w:id="-1" w:val="840"/></w:rPr></w:rPrDefault></w:docDefaults>
          <w:style w:type="character" w:styleId="fitted"><w:rPr><w:fitText w:id="-931340544" w:val="840"></w:fitText></w:rPr></w:style>
        </w:styles>"#;
        let part = StylesPart::from_xml(xml).unwrap();
        assert_eq!(
            part.doc_default_run_properties()
                .unwrap()
                .fit_text
                .as_ref()
                .unwrap()
                .id
                .map(i64::from),
            Some(-1)
        );
        assert_eq!(
            part.character_styles
                .get("fitted")
                .unwrap()
                .run_properties
                .as_ref()
                .unwrap()
                .fit_text
                .as_ref()
                .unwrap()
                .id
                .map(i64::from),
            Some(-931340544)
        );
    }

    #[test]
    fn parses_run_kerning_and_text_scale_in_styles() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault>
      <w:rPr>
        <w:kern w:val="24"/>
        <w:w w:val="105"/>
        <w:fitText w:id="10" w:val="480"/>
        <w:lang w:val="en-US" w:eastAsia="ja-JP"/>
      </w:rPr>
    </w:rPrDefault>
  </w:docDefaults>
  <w:style w:type="paragraph" w:styleId="jpStyle">
    <w:rPr>
      <w:kern w:val="28"/>
      <w:w w:val="110"/>
      <w:fitText w:id="20" w:val="720"/>
      <w:lang w:val="en-GB" w:eastAsia="zh-TW"/>
    </w:rPr>
  </w:style>
</w:styles>
"#;

        let part = StylesPart::from_xml(xml).expect("parse styles");
        let defaults = part.doc_default_run_properties().expect("doc defaults");
        assert_eq!(defaults.kerning, Some(12.0));
        assert_eq!(defaults.text_scale, Some(105.0));
        assert_eq!(defaults.language.as_deref(), Some("en-US"));
        assert_eq!(defaults.east_asia_language.as_deref(), Some("ja-JP"));
        assert_eq!(
            defaults
                .fit_text
                .as_ref()
                .map(|fit_text| (fit_text.id, fit_text.val)),
            Some((Some(10), 480))
        );

        let style = part.get_style("jpStyle").expect("style");
        let run_props = style.run_properties.as_ref().expect("run props");
        assert_eq!(run_props.kerning, Some(14.0));
        assert_eq!(run_props.text_scale, Some(110.0));
        assert_eq!(run_props.language.as_deref(), Some("en-GB"));
        assert_eq!(run_props.east_asia_language.as_deref(), Some("zh-TW"));
        assert_eq!(
            run_props
                .fit_text
                .as_ref()
                .map(|fit_text| (fit_text.id, fit_text.val)),
            Some((Some(20), 720))
        );
    }

    #[test]
    fn underline_without_val_does_not_override_style_hierarchy() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault><w:rPr><w:u w:val="none"/></w:rPr></w:rPrDefault>
  </w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal">
    <w:rPr><w:u w:color="000000"/></w:rPr>
  </w:style>
  <w:style w:type="character" w:styleId="Hyperlink">
    <w:rPr><w:u w:val="single" w:color="0000FF"/></w:rPr>
  </w:style>
</w:styles>
"#;

        let part = StylesPart::from_xml(xml).expect("parse styles");
        let normal = part
            .resolve_effective_run_properties(Some("Normal"), None)
            .expect("resolve normal style");
        let hyperlink_hint = RunProperties {
            style: Some(Rc::from("Hyperlink")),
            ..Default::default()
        };
        let hyperlink = part
            .resolve_effective_run_properties(Some("Normal"), Some(&hyperlink_hint))
            .expect("resolve hyperlink style");

        assert_eq!(normal.underline, Some(false));
        assert_eq!(hyperlink.underline, Some(true));
    }

    /// A style-declared `w:u` carries its kind and color through effective
    /// resolution (§17.3.2.40) — a wave-underlined character style must not
    /// flatten to a plain single rule.
    #[test]
    fn style_underline_kind_and_color_survive_effective_resolution() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal">
    <w:rPr/>
  </w:style>
  <w:style w:type="character" w:styleId="Wavy">
    <w:rPr><w:u w:val="wave" w:color="00B050"/></w:rPr>
  </w:style>
</w:styles>
"#;
        let part = StylesPart::from_xml(xml).expect("parse styles");
        let hint = RunProperties {
            style: Some(Rc::from("Wavy")),
            ..Default::default()
        };
        let resolved = part
            .resolve_effective_run_properties(Some("Normal"), Some(&hint))
            .expect("resolve wavy style");
        assert_eq!(resolved.underline, Some(true));
        assert_eq!(resolved.underline_kind.as_deref(), Some("wave"));
        assert_eq!(resolved.underline_color.as_deref(), Some("00B050"));
    }

    #[test]
    fn resolves_character_style_run_properties_across_based_on_chain() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="character" w:styleId="BaseChar">
    <w:rPr>
      <w:rFonts w:ascii="ＭＳ 明朝" w:hAnsi="ＭＳ 明朝" w:eastAsia="ＭＳ 明朝"/>
      <w:sz w:val="21"/>
    </w:rPr>
  </w:style>
  <w:style w:type="character" w:styleId="Hyperlink.0">
    <w:basedOn w:val="BaseChar"/>
    <w:rPr>
      <w:color w:val="0000FF"/>
    </w:rPr>
  </w:style>
</w:styles>
"#;

        let part = StylesPart::from_xml(xml).expect("parse styles");
        let run_props = part
            .resolve_character_run_properties("Hyperlink.0")
            .expect("resolve character style");
        let fonts = run_props.fonts.as_ref().expect("character style fonts");
        assert_eq!(fonts.ascii.as_deref(), Some("ＭＳ 明朝"));
        assert_eq!(fonts.east_asia.as_deref(), Some("ＭＳ 明朝"));
        assert_eq!(run_props.font_size, Some(10.5));
        assert_eq!(run_props.color.as_deref(), Some("#0000FF"));
    }

    #[test]
    fn normalizes_inherited_style_colors_for_canvas_rendering() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults><w:rPrDefault><w:rPr><w:color w:val="a1b2c3"/></w:rPr></w:rPrDefault></w:docDefaults>
  <w:style w:type="paragraph" w:styleId="Title">
    <w:rPr><w:color w:val="000000"/></w:rPr>
  </w:style>
  <w:style w:type="paragraph" w:styleId="DerivedTitle"><w:basedOn w:val="Title"/></w:style>
  <w:style w:type="character" w:styleId="Automatic"><w:rPr><w:color w:val="auto"/></w:rPr></w:style>
</w:styles>
"#;
        let part = StylesPart::from_xml(xml).expect("parse styles");
        let defaults = part
            .resolve_effective_run_properties(None, None)
            .expect("default run properties");
        assert_eq!(defaults.color.as_deref(), Some("#A1B2C3"));
        let inherited = part
            .resolve_effective_run_properties(Some("DerivedTitle"), None)
            .expect("inherited title properties");
        assert_eq!(inherited.color.as_deref(), Some("#000000"));
        let automatic = part
            .resolve_character_run_properties("Automatic")
            .expect("automatic character properties");
        assert_eq!(automatic.color, None);
    }

    #[test]
    fn resolves_character_style_run_properties_until_based_on_cycle() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="character" w:styleId="CycleA">
    <w:basedOn w:val="CycleB"/>
    <w:rPr><w:b/></w:rPr>
  </w:style>
  <w:style w:type="character" w:styleId="CycleB">
    <w:basedOn w:val="CycleA"/>
    <w:rPr><w:i/></w:rPr>
  </w:style>
</w:styles>
"#;

        let part = StylesPart::from_xml(xml).expect("parse styles");
        let run_props = part
            .resolve_character_run_properties("CycleA")
            .expect("resolve character style");
        assert_eq!(run_props.bold, Some(true));
        assert_eq!(run_props.italic, Some(true));
    }

    #[test]
    fn resolves_character_style_run_properties_beyond_inline_chain_capacity() {
        let mut xml = String::from(
            r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">"#,
        );
        for index in 0..10 {
            let based_on = if index < 9 {
                format!(r#"<w:basedOn w:val="Deep{}"/>"#, index + 1)
            } else {
                String::new()
            };
            let run_properties = match index {
                0 => "<w:rPr><w:b/></w:rPr>",
                9 => "<w:rPr><w:i/></w:rPr>",
                _ => "",
            };
            xml.push_str(&format!(
                r#"<w:style w:type="character" w:styleId="Deep{index}">{based_on}{run_properties}</w:style>"#,
            ));
        }
        xml.push_str("</w:styles>");

        let part = StylesPart::from_xml(&xml).expect("parse styles");
        let run_props = part
            .resolve_character_run_properties("Deep0")
            .expect("resolve deep character style");
        assert_eq!(run_props.bold, Some(true));
        assert_eq!(run_props.italic, Some(true));
    }

    #[test]
    fn resolves_effective_run_properties_in_word_cascade_order() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults><w:rPrDefault><w:rPr><w:i/><w:color w:val="111111"/></w:rPr></w:rPrDefault></w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal">
    <w:rPr><w:u w:val="single"/><w:color w:val="222222"/></w:rPr>
  </w:style>
  <w:style w:type="character" w:styleId="Strong">
    <w:rPr><w:b/><w:color w:val="333333"/></w:rPr>
  </w:style>
</w:styles>
"#;

        let part = StylesPart::from_xml(xml).expect("parse styles");
        let direct = RunProperties {
            style: Some(Rc::from("Strong")),
            italic: Some(false),
            color: Some(Rc::from("#444444")),
            ..Default::default()
        };
        let resolved = part
            .resolve_effective_run_properties(None, Some(&direct))
            .expect("resolved cascade");

        assert_eq!(resolved.underline, Some(true));
        assert_eq!(resolved.bold, Some(true));
        assert_eq!(resolved.italic, Some(false));
        assert_eq!(resolved.color.as_deref(), Some("#444444"));
        assert_eq!(resolved.style.as_deref(), Some("Strong"));
    }

    #[test]
    fn direct_run_fonts_without_hint_preserve_authored_provenance() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults>
    <w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hint="eastAsia"/></w:rPr></w:rPrDefault>
  </w:docDefaults>
</w:styles>
"#;
        let part = StylesPart::from_xml(xml).expect("parse styles");
        let direct_without_hint = RunProperties {
            fonts: Some(RunFonts {
                ascii: Some(Rc::from("Arial")),
                ..RunFonts::default()
            }),
            authored_east_asia_font_hint: Some(false),
            authored_east_asia_font_name: Some(false),
            ..RunProperties::default()
        };
        let inherited = part
            .resolve_effective_run_properties(None, Some(&direct_without_hint))
            .expect("resolved inherited hint");

        assert_eq!(
            inherited
                .fonts
                .as_ref()
                .and_then(|fonts| fonts.hint.clone()),
            Some(FontHint::EastAsia)
        );
        assert_eq!(
            inherited
                .fonts
                .as_ref()
                .and_then(|fonts| fonts.ascii.as_deref()),
            Some("Arial")
        );
        assert_eq!(inherited.authored_east_asia_font_hint, Some(false));
        assert_eq!(inherited.authored_east_asia_font_name, Some(false));

        let explicit_default = RunProperties {
            fonts: Some(RunFonts {
                hint: Some(FontHint::Default),
                ..RunFonts::default()
            }),
            ..RunProperties::default()
        };
        let overridden = part
            .resolve_effective_run_properties(None, Some(&explicit_default))
            .expect("resolved explicit default hint");
        assert_eq!(
            overridden
                .fonts
                .as_ref()
                .and_then(|fonts| fonts.hint.clone()),
            Some(FontHint::Default)
        );

        let explicit_complex = RunProperties {
            fonts: Some(RunFonts {
                hint: Some(FontHint::Complex),
                ..RunFonts::default()
            }),
            authored_east_asia_font_hint: Some(false),
            authored_east_asia_font_name: Some(false),
            ..RunProperties::default()
        };
        let overridden = part
            .resolve_effective_run_properties(None, Some(&explicit_complex))
            .expect("resolved explicit complex hint");
        assert_eq!(
            overridden
                .fonts
                .as_ref()
                .and_then(|fonts| fonts.hint.clone()),
            Some(FontHint::Complex)
        );
    }

    #[test]
    fn resolves_default_paragraph_style_id_from_flag() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="Custom">
    <w:name w:val="Custom"/>
  </w:style>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal.0">
    <w:name w:val="Normal"/>
  </w:style>
</w:styles>
"#;
        let part = StylesPart::from_xml(xml).expect("parse styles");
        assert_eq!(part.default_paragraph_style_id(), Some("Normal.0"));
        assert!(part
            .get_style("Normal.0")
            .is_some_and(|style| style.is_default));
    }

    #[test]
    fn the_last_style_named_normal_defines_every_normal_style() {
        // yokohama_table: Word for Mac draws an unstyled empty paragraph, and
        // one styled `Normal.0`, in the later Normal (Yu Mincho 10.5pt); with
        // the two definitions swapped, both draw in Times New Roman 12pt.
        let style = |id: &str, default: bool, size: u32| {
            format!(
                r#"<w:style w:type="paragraph"{} w:styleId="{id}"><w:name w:val="Normal"/><w:rPr><w:sz w:val="{size}"/></w:rPr></w:style>"#,
                if default { r#" w:default="1""# } else { "" }
            )
        };
        let styles = |first: String, second: String| {
            let xml = format!(
                r#"<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{first}{second}</w:styles>"#
            );
            StylesPart::from_xml(&xml).expect("parse styles")
        };
        let size_of = |part: &StylesPart, id: &str| {
            part.get_style(id)
                .and_then(|style| style.run_properties.as_ref())
                .and_then(|props| props.font_size)
        };
        let part = styles(style("Normal", true, 24), style("Normal.0", false, 21));
        assert_eq!(part.default_paragraph_style_id(), Some("Normal"));
        assert_eq!(size_of(&part, "Normal"), Some(10.5));
        assert_eq!(size_of(&part, "Normal.0"), Some(10.5));
        let part = styles(style("Normal.0", false, 21), style("Normal", true, 24));
        assert_eq!(size_of(&part, "Normal"), Some(12.0));
        assert_eq!(size_of(&part, "Normal.0"), Some(12.0));
    }

    #[test]
    fn falls_back_to_named_normal_when_default_flag_missing() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:styleId="Normal">
    <w:name w:val="Normal"/>
  </w:style>
</w:styles>
"#;
        let part = StylesPart::from_xml(xml).expect("parse styles");
        assert_eq!(part.default_paragraph_style_id(), Some("Normal"));
    }

    #[test]
    fn table_style_paragraph_properties_skip_conditional_formats() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="table" w:styleId="TableNormal"><w:name w:val="Normal Table"/></w:style>
  <w:style w:type="table" w:styleId="TableGrid">
    <w:basedOn w:val="TableNormal"/>
    <w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr>
    <w:rPr><w:sz w:val="40"/></w:rPr>
    <w:tblStylePr w:type="firstRow"><w:pPr><w:spacing w:before="480"/></w:pPr></w:tblStylePr>
  </w:style>
</w:styles>"#;
        let styles = StylesPart::from_xml(xml).expect("styles");
        let spacing = styles
            .resolve_table_paragraph_properties("TableGrid")
            .and_then(|properties| properties.spacing)
            .expect("table style spacing");
        assert_eq!(spacing.after, Some(0.0));
        assert_eq!(spacing.line, Some(1.0));
        assert_eq!(spacing.before, None);
        assert!(styles.resolve_paragraph_properties("TableGrid").is_none());
        assert!(styles
            .resolve_table_paragraph_properties("TableNormal")
            .is_none());
    }

    #[test]
    fn parses_and_resolves_document_table_style_gallery() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="table" w:default="1" w:styleId="TableNormal">
    <w:name w:val="Normal Table"/>
  </w:style>
  <w:style w:type="table" w:styleId="GridBase">
    <w:name w:val="Grid Base"/>
    <w:tblPr>
      <w:tblBorders>
        <w:top w:val="single" w:sz="8" w:color="102030"/>
        <w:insideH w:val="dashed" w:sz="4" w:color="auto"/>
      </w:tblBorders>
      <w:tblCellMar>
        <w:top w:w="40" w:type="dxa"/>
        <w:start w:w="120" w:type="dxa"/>
      </w:tblCellMar>
    </w:tblPr>
    <w:tcPr><w:shd w:fill="F0F0F0"/></w:tcPr>
    <w:tblStylePr w:type="firstRow">
      <w:rPr><w:b/><w:color w:val="FFFFFF"/></w:rPr>
      <w:tcPr><w:shd w:fill="112233"/></w:tcPr>
    </w:tblStylePr>
    <w:tblStylePr w:type="band1Horz">
      <w:tcPr><w:shd w:fill="DDEEFF"/></w:tcPr>
    </w:tblStylePr>
  </w:style>
  <w:style w:type="table" w:styleId="GridChild">
    <w:name w:val="Grid Child"/>
    <w:basedOn w:val="GridBase"/>
    <w:tblPr>
      <w:tblBorders><w:bottom w:val="double" w:sz="12" w:color="445566"/></w:tblBorders>
      <w:tblCellMar><w:end w:w="80" w:type="dxa"/></w:tblCellMar>
    </w:tblPr>
    <w:tblStylePr w:type="firstRow">
      <w:rPr><w:b w:val="0"/></w:rPr>
    </w:tblStylePr>
    <w:tblStylePr w:type="band2Horz">
      <w:tcPr><w:shd w:fill="CCDDEE"/></w:tcPr>
    </w:tblStylePr>
  </w:style>
</w:styles>
"#;

        let part = StylesPart::from_xml(xml).expect("parse table styles");
        assert_eq!(part.resolve_table_style_id("GridChild"), Some("GridChild"));
        assert_eq!(
            part.resolve_table_style_id(" grid child "),
            Some("GridChild")
        );

        let gallery = part.table_style_infos();
        assert_eq!(
            gallery
                .iter()
                .map(|style| (style.style_id.as_str(), style.name.as_str()))
                .collect::<Vec<_>>(),
            vec![("GridBase", "Grid Base"), ("GridChild", "Grid Child")],
            "TableNormal is not a selectable gallery entry and source order is stable"
        );

        let child = part
            .resolve_table_style("GridChild")
            .expect("resolve basedOn chain");
        assert_eq!(child.fill.as_deref(), Some("#F0F0F0"));
        assert_eq!(child.band1_fill.as_deref(), Some("#DDEEFF"));
        assert_eq!(child.band2_fill.as_deref(), Some("#CCDDEE"));
        let first = child.first_row.expect("first-row formatting");
        assert_eq!(first.fill.as_deref(), Some("#112233"));
        assert_eq!(first.color.as_deref(), Some("#FFFFFF"));
        assert_eq!(
            first.bold,
            Some(false),
            "child explicit off overrides parent on"
        );
        let borders = child.borders.expect("inherited borders");
        assert_eq!(
            borders.top.as_ref().and_then(|edge| edge.color.as_deref()),
            Some("#102030")
        );
        assert_eq!(
            borders.bottom.as_ref().and_then(|edge| edge.val.as_deref()),
            Some("double")
        );
        assert_eq!(
            borders
                .inside_h
                .as_ref()
                .and_then(|edge| edge.color.as_deref()),
            None,
            "auto color remains a downstream fallback"
        );
        let margins = child.cell_margins.expect("inherited margins");
        assert_eq!(margins.top, Some(2.0));
        assert_eq!(margins.start, Some(6.0));
        assert_eq!(margins.end, Some(4.0));
    }

    #[test]
    fn table_style_gallery_omits_normal_table_with_localized_id() {
        // Japanese Word packages give built-in styles opaque ids and keep the
        // English built-in name in `w:name`.
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="table" w:default="1" w:styleId="a1">
    <w:name w:val="Normal Table"/>
    <w:tblPr>
      <w:tblCellMar>
        <w:top w:w="0" w:type="dxa"/>
        <w:left w:w="108" w:type="dxa"/>
      </w:tblCellMar>
    </w:tblPr>
  </w:style>
  <w:style w:type="table" w:styleId="af">
    <w:name w:val="Table Grid"/>
    <w:basedOn w:val="a1"/>
    <w:tblPr>
      <w:tblBorders><w:top w:val="single" w:sz="4" w:color="auto"/></w:tblBorders>
    </w:tblPr>
  </w:style>
</w:styles>
"#;

        let part = StylesPart::from_xml(xml).expect("parse table styles");
        assert_eq!(
            part.table_style_infos()
                .iter()
                .map(|style| (style.style_id.as_str(), style.name.as_str()))
                .collect::<Vec<_>>(),
            vec![("af", "Table Grid")],
            "Normal Table is matched by w:name when its id is localized"
        );
        assert_eq!(part.resolve_table_style_id("a1"), Some("a1"));
        assert_eq!(part.resolve_table_style_id("Normal Table"), Some("a1"));
        assert_eq!(part.default_table_style_id(), Some("a1"));
        let default = part.default_table_style().expect("default table style");
        assert_eq!(
            default.cell_margins.and_then(|margins| margins.top),
            Some(0.0)
        );
        let grid = part
            .resolve_table_style("Table Grid")
            .expect("gallery style still inherits Normal Table");
        assert_eq!(
            grid.cell_margins.and_then(|margins| margins.left),
            Some(5.4)
        );
    }

    #[test]
    fn parses_every_conditional_region_band_sizes_and_inherited_condition_edges() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="table" w:styleId="Base">
    <w:tblPr>
      <w:tblStyleRowBandSize w:val="2"/>
      <w:tblStyleColBandSize w:val="1"/>
    </w:tblPr>
    <w:tblStylePr w:type="wholeTable">
      <w:tcPr><w:shd w:fill="999999"/></w:tcPr>
    </w:tblStylePr>
    <w:tblStylePr w:type="lastCol">
      <w:rPr><w:i/><w:color w:val="00FF00"/></w:rPr>
      <w:tcPr>
        <w:tcBorders>
          <w:left w:val="single" w:sz="8" w:color="AA0000"/>
          <w:insideH w:val="nil"/>
        </w:tcBorders>
        <w:shd w:fill="ABCDEF"/>
      </w:tcPr>
    </w:tblStylePr>
    <w:tblStylePr w:type="seCell">
      <w:tcPr><w:shd w:fill="0000FF"/></w:tcPr>
    </w:tblStylePr>
  </w:style>
  <w:style w:type="table" w:styleId="Derived">
    <w:basedOn w:val="Base"/>
    <w:tblPr><w:tblStyleColBandSize w:val="3"/></w:tblPr>
    <w:tblStylePr w:type="lastCol">
      <w:rPr><w:b/></w:rPr>
      <w:tcPr><w:tcBorders><w:left w:val="double" w:sz="4"/></w:tcBorders></w:tcPr>
    </w:tblStylePr>
  </w:style>
</w:styles>
"#;
        let part = StylesPart::from_xml(xml).expect("parse table styles");
        let style = part.resolve_table_style("Derived").expect("resolve style");
        assert_eq!(style.row_band_size, Some(2));
        assert_eq!(style.col_band_size, Some(3));
        assert!(
            style.fill.is_none(),
            "Word discards wholeTable conditional properties"
        );
        let last_col = &style.conditional[&TableStyleCondition::LastCol];
        assert_eq!(last_col.fill.as_deref(), Some("#ABCDEF"));
        assert_eq!(last_col.italic, Some(true));
        assert_eq!(last_col.bold, Some(true));
        assert_eq!(last_col.color.as_deref(), Some("#00FF00"));
        let borders = last_col.borders.as_ref().expect("conditional borders");
        assert_eq!(
            borders.left.as_ref().and_then(|edge| edge.val.as_deref()),
            Some("double"),
            "the derived style replaces the inherited edge"
        );
        assert_eq!(
            borders
                .inside_h
                .as_ref()
                .and_then(|edge| edge.val.as_deref()),
            Some("nil")
        );
        assert_eq!(
            style.conditional[&TableStyleCondition::SeCell]
                .fill
                .as_deref(),
            Some("#0000FF")
        );
        assert_eq!(style.conditional.len(), 2);
    }

    #[test]
    fn table_style_cycles_are_finite_and_keep_local_properties() {
        let xml = r#"
<w:styles xmlns:w="urn:w">
  <w:style w:type="table" w:styleId="A">
    <w:basedOn w:val="B"/><w:tcPr><w:shd w:fill="AAAAAA"/></w:tcPr>
  </w:style>
  <w:style w:type="table" w:styleId="B">
    <w:basedOn w:val="A"/>
    <w:tblStylePr w:type="band1Horz"><w:tcPr><w:shd w:fill="BBBBBB"/></w:tcPr></w:tblStylePr>
  </w:style>
</w:styles>
"#;
        let part = StylesPart::from_xml(xml).expect("parse cyclic styles");
        let resolved = part
            .resolve_table_style("A")
            .expect("local style survives cycle");
        assert_eq!(resolved.fill.as_deref(), Some("#AAAAAA"));
        assert_eq!(resolved.band1_fill.as_deref(), Some("#BBBBBB"));
        assert_eq!(part.table_style_infos().len(), 2);
    }

    #[test]
    fn default_table_style_supplies_word_normal_table_cell_margins() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="paragraph" w:default="1" w:styleId="a">
    <w:name w:val="Normal"/>
  </w:style>
  <w:style w:type="table" w:default="1" w:styleId="a1">
    <w:name w:val="Normal Table"/>
    <w:tblPr>
      <w:tblInd w:w="0" w:type="dxa"/>
      <w:tblCellMar>
        <w:top w:w="0" w:type="dxa"/>
        <w:left w:w="108" w:type="dxa"/>
        <w:bottom w:w="0" w:type="dxa"/>
        <w:right w:w="108" w:type="dxa"/>
      </w:tblCellMar>
    </w:tblPr>
  </w:style>
  <w:style w:type="table" w:styleId="Grid">
    <w:name w:val="Table Grid"/>
  </w:style>
</w:styles>
"#;

        let part = StylesPart::from_xml(xml).expect("parse styles");
        assert_eq!(part.default_table_style_id(), Some("a1"));
        let display = part.default_table_style().expect("default table style");
        let margins = display
            .cell_margins
            .expect("Normal Table declares tblCellMar");
        assert_eq!(margins.top, Some(0.0));
        assert_eq!(margins.bottom, Some(0.0));
        assert_eq!(margins.left, Some(5.4));
        assert_eq!(margins.right, Some(5.4));
    }

    #[test]
    fn default_table_style_requires_the_default_marker() {
        let xml = r#"
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:style w:type="table" w:styleId="Grid">
    <w:name w:val="Table Grid"/>
    <w:tblPr>
      <w:tblCellMar><w:top w:w="40" w:type="dxa"/></w:tblCellMar>
    </w:tblPr>
  </w:style>
</w:styles>
"#;

        let part = StylesPart::from_xml(xml).expect("parse styles");
        assert_eq!(part.default_table_style_id(), None);
        assert!(part.default_table_style().is_none());
    }
}
