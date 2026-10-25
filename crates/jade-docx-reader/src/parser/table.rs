use super::XmlEventSource;
use super::*;
use quick_xml::events::{attributes::Attributes, Event};

pub(super) enum ContainerContext {
    Table(Box<Table>),
    TableRow(TableRow),
    TableCell(TableCell),
}

impl DocumentParser {
    pub(super) fn push_block(
        document: &mut Document,
        container_stack: &mut [ContainerContext],
        block: BodyElement,
    ) {
        if let Some(cell_ctx) = container_stack.iter_mut().rev().find_map(|ctx| {
            if let ContainerContext::TableCell(cell) = ctx {
                Some(cell)
            } else {
                None
            }
        }) {
            cell_ctx.blocks.push(block);
            return;
        }

        if let Some(body) = document.body.as_mut() {
            body.elements.push(block);
        }
    }

    pub(super) fn push_paragraph(
        document: &mut Document,
        container_stack: &mut [ContainerContext],
        mut paragraph: Paragraph,
        props: Option<ParagraphProperties>,
        comment_ranges: Vec<CommentRange>,
        comment_markers: Vec<CommentMarker>,
    ) {
        paragraph.properties = props.and_then(|p| {
            if Self::paragraph_props_is_empty(&p) {
                None
            } else {
                Some(p)
            }
        });
        if !comment_ranges.is_empty() {
            paragraph.comment_ranges = comment_ranges;
        }
        if !comment_markers.is_empty() {
            paragraph.comment_markers = comment_markers;
        }
        Self::push_block(
            document,
            container_stack,
            BodyElement::from_paragraph(paragraph),
        );
    }

    pub(super) fn paragraph_props_is_empty(props: &ParagraphProperties) -> bool {
        props.para_id.is_none()
            && props.style.is_none()
            && props
                .mark_run_properties
                .as_ref()
                .is_none_or(RunProperties::is_empty)
            && props.mark_revision.is_none()
            && props.alignment.is_none()
            && props.bidi.is_none()
            && props.numbering.is_none()
            && props.section.is_none()
            && props.drop_cap.is_none()
            && props.frame.is_none()
            && props.adjust_right_indent.is_none()
            && props.snap_to_grid.is_none()
            && props.widow_control.is_none()
            && props.suppress_auto_hyphens.is_none()
            && props.page_break_before.is_none()
            && props.suppress_line_numbers.is_none()
            && props.keep_lines.is_none()
            && props.keep_next.is_none()
            && props.text_alignment.is_none()
            && props.mirror_indents.is_none()
            && props.outline_level.is_none()
            && props.east_asian_settings.as_ref().is_none_or(|settings| {
                !settings.use_east_asian_layout
                    && settings.kinsoku.is_none()
                    && settings.word_wrap.is_none()
                    && settings.overflow_punctuation.is_none()
                    && settings.top_line_punctuation.is_none()
                    && !settings.balance_single_byte_double_byte_width
                    && settings.auto_space_de.is_none()
                    && settings.auto_space_dn.is_none()
                    && settings.character_spacing_control.is_none()
                    && settings.no_line_break_before.is_empty()
                    && settings.no_line_break_after.is_empty()
            })
            && props.indent.as_ref().is_none_or(Self::indent_is_empty)
            && props.spacing.as_ref().is_none_or(Self::spacing_is_empty)
            && props.tabs.is_empty()
            && props.shading.as_ref().is_none_or(|s| s.is_empty())
            && props.borders.as_ref().is_none_or(|b| b.is_empty())
            && props.contextual_spacing.is_none()
    }

    pub(super) fn indent_is_empty(indent: &Indent) -> bool {
        indent.left.is_none()
            && indent.right.is_none()
            && indent.start.is_none()
            && indent.end.is_none()
            && indent.first_line.is_none()
            && indent.hanging.is_none()
            && indent.left_chars.is_none()
            && indent.right_chars.is_none()
            && indent.start_chars.is_none()
            && indent.end_chars.is_none()
            && indent.first_line_chars.is_none()
            && indent.hanging_chars.is_none()
    }

    pub(super) fn spacing_is_empty(spacing: &Spacing) -> bool {
        spacing.before.is_none()
            && spacing.after.is_none()
            && spacing.before_lines.is_none()
            && spacing.after_lines.is_none()
            && spacing.line.is_none()
            && spacing.line_rule.is_none()
    }

    pub(super) fn parse_table_grid(
        &self,
        reader: &mut dyn XmlEventSource,
    ) -> Result<Vec<TableColumn>, Box<dyn std::error::Error>> {
        let mut columns = Vec::new();
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => match e.name().as_ref() {
                    b"w:gridCol" => {
                        let width = Self::parse_grid_column_width(e.attributes());
                        columns.push(TableColumn { width });
                        self.skip_element(reader, b"w:gridCol")?;
                    }
                    _ => {
                        self.skip_element(reader, e.name().as_ref())?;
                    }
                },
                Ok(Event::Empty(ref e)) => {
                    if e.name().as_ref() == b"w:gridCol" {
                        let width = Self::parse_grid_column_width(e.attributes());
                        columns.push(TableColumn { width });
                    }
                }
                Ok(Event::End(ref e)) => {
                    if e.name().as_ref() == b"w:tblGrid" {
                        break;
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("XML parsing error: {}", err).into()),
                _ => {}
            }
        }

        Ok(columns)
    }

    pub(super) fn parse_table_properties(
        &self,
        reader: &mut dyn XmlEventSource,
        table: &mut Table,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => match e.name().as_ref() {
                    b"w:tblStyle" => {
                        table.style_name = Self::read_val_attribute(e.attributes());
                        Self::mark_position_before_style(table);
                        self.skip_element(reader, b"w:tblStyle")?;
                    }
                    b"w:tblLook" => {
                        Self::parse_table_look(table, e.attributes());
                        self.skip_element(reader, b"w:tblLook")?;
                    }
                    b"w:tblStyleColBandSize" => {
                        table.col_band_size = Self::read_int_attribute(e.attributes());
                        self.skip_element(reader, b"w:tblStyleColBandSize")?;
                    }
                    b"w:tblStyleRowBandSize" => {
                        table.row_band_size = Self::read_int_attribute(e.attributes());
                        self.skip_element(reader, b"w:tblStyleRowBandSize")?;
                    }
                    b"w:tblW" => {
                        if let Some(width) = Self::parse_table_width(e.attributes()) {
                            table.style.insert("width".to_string(), width);
                        }
                        self.skip_element(reader, b"w:tblW")?;
                    }
                    b"w:tblInd" => {
                        let (indent, ignored_type) = Self::parse_table_indent(e.attributes());
                        if let Some(indent) = indent {
                            table.style.insert("margin-left".to_string(), indent);
                        }
                        if let Some(kind) = ignored_type {
                            table
                                .style
                                .insert("table-indent-width-type".to_string(), kind);
                        }
                        self.skip_element(reader, b"w:tblInd")?;
                    }
                    b"w:jc" => {
                        if let Some(alignment) = Self::read_val_attribute(e.attributes()) {
                            let normalized = Self::normalize_paragraph_alignment(&alignment)
                                .to_ascii_lowercase();
                            table.style.insert("text-align".to_string(), normalized);
                        }
                        self.skip_element(reader, b"w:jc")?;
                    }
                    b"w:tblpPr" => {
                        Self::parse_table_position_properties(table, e.attributes());
                        self.skip_element(reader, b"w:tblpPr")?;
                    }
                    b"w:tblOverlap" => {
                        if let Some(value) = Self::read_val_attribute(e.attributes()) {
                            let value = value.trim().to_ascii_lowercase();
                            if matches!(value.as_str(), "never" | "overlap") {
                                table.style.insert("table-overlap".to_string(), value);
                            }
                        }
                        self.skip_element(reader, b"w:tblOverlap")?;
                    }
                    b"w:hidden" => {
                        table
                            .style
                            .insert("display".to_string(), "none".to_string());
                        self.skip_element(reader, b"w:hidden")?;
                    }
                    b"w:shd" => {
                        if let Some(color) = Self::read_color_attribute(e.attributes(), b"w:fill") {
                            table
                                .cell_style
                                .insert("background-color".to_string(), color);
                        }
                        self.skip_element(reader, b"w:shd")?;
                    }
                    b"w:tblBorders" => {
                        let borders = Self::parse_table_borders(reader)?;
                        if !borders.is_empty() {
                            table.borders = Some(borders);
                        }
                    }
                    b"w:tblCellMar" => {
                        let margins = Self::parse_cell_margins(reader, b"w:tblCellMar")?;
                        if !margins.is_empty() {
                            table.cell_margins = Some(margins);
                        }
                    }
                    b"w:tblCellSpacing" => {
                        table.cell_spacing = Self::table_cell_spacing_value(e.attributes());
                        self.skip_element(reader, b"w:tblCellSpacing")?;
                    }
                    b"w:tblLayout" => {
                        table.table_layout = Self::parse_table_layout_type(e.attributes());
                        self.skip_element(reader, b"w:tblLayout")?;
                    }
                    b"w:bidiVisual" => {
                        table.bidi_visual =
                            parse_on_off(Self::read_val_attribute(e.attributes()), true);
                        self.skip_element(reader, b"w:bidiVisual")?;
                    }
                    _ => self.skip_element(reader, e.name().as_ref())?,
                },
                Ok(Event::Empty(ref e)) => match e.name().as_ref() {
                    b"w:tblStyle" => {
                        table.style_name = Self::read_val_attribute(e.attributes());
                        Self::mark_position_before_style(table);
                    }
                    b"w:tblLook" => {
                        Self::parse_table_look(table, e.attributes());
                    }
                    b"w:tblStyleColBandSize" => {
                        table.col_band_size = Self::read_int_attribute(e.attributes());
                    }
                    b"w:tblStyleRowBandSize" => {
                        table.row_band_size = Self::read_int_attribute(e.attributes());
                    }
                    b"w:tblW" => {
                        if let Some(width) = Self::parse_table_width(e.attributes()) {
                            table.style.insert("width".to_string(), width);
                        }
                    }
                    b"w:tblInd" => {
                        let (indent, ignored_type) = Self::parse_table_indent(e.attributes());
                        if let Some(indent) = indent {
                            table.style.insert("margin-left".to_string(), indent);
                        }
                        if let Some(kind) = ignored_type {
                            table
                                .style
                                .insert("table-indent-width-type".to_string(), kind);
                        }
                    }
                    b"w:jc" => {
                        if let Some(alignment) = Self::read_val_attribute(e.attributes()) {
                            let normalized = Self::normalize_paragraph_alignment(&alignment)
                                .to_ascii_lowercase();
                            table.style.insert("text-align".to_string(), normalized);
                        }
                    }
                    b"w:tblpPr" => {
                        Self::parse_table_position_properties(table, e.attributes());
                    }
                    b"w:tblOverlap" => {
                        if let Some(value) = Self::read_val_attribute(e.attributes()) {
                            let value = value.trim().to_ascii_lowercase();
                            if matches!(value.as_str(), "never" | "overlap") {
                                table.style.insert("table-overlap".to_string(), value);
                            }
                        }
                    }
                    b"w:hidden" => {
                        table
                            .style
                            .insert("display".to_string(), "none".to_string());
                    }
                    b"w:shd" => {
                        if let Some(color) = Self::read_color_attribute(e.attributes(), b"w:fill") {
                            table
                                .cell_style
                                .insert("background-color".to_string(), color);
                        }
                    }
                    b"w:tblCellSpacing" => {
                        table.cell_spacing = Self::table_cell_spacing_value(e.attributes());
                    }
                    b"w:tblLayout" => {
                        table.table_layout = Self::parse_table_layout_type(e.attributes());
                    }
                    b"w:bidiVisual" => {
                        table.bidi_visual =
                            parse_on_off(Self::read_val_attribute(e.attributes()), true);
                    }
                    _ => {}
                },
                Ok(Event::End(ref e)) => {
                    if e.name().as_ref() == b"w:tblPr" {
                        break;
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("XML parsing error: {}", err).into()),
                _ => {}
            }
        }

        Ok(())
    }

    /// Parses `w:tblLook`. Word reads the hexadecimal `w:val` only when none
    /// of the explicit on/off attributes are present (MS-OI29500
    /// §17.4.55(c)); once any attribute is present, an omitted one is off.
    fn parse_table_look(table: &mut Table, attributes: Attributes<'_>) {
        let mut raw_value = None;
        let mut explicit = TableLook::default();
        let mut has_explicit = false;
        for attr in attributes.flatten() {
            let value = String::from_utf8_lossy(&attr.value).to_string();
            let slot = match attr.key.as_ref() {
                b"w:val" => {
                    raw_value = Some(value);
                    continue;
                }
                b"w:firstRow" => &mut explicit.first_row,
                b"w:lastRow" => &mut explicit.last_row,
                b"w:firstColumn" => &mut explicit.first_column,
                b"w:lastColumn" => &mut explicit.last_column,
                b"w:noHBand" => &mut explicit.no_h_band,
                b"w:noVBand" => &mut explicit.no_v_band,
                _ => continue,
            };
            *slot = Some(parse_on_off(Some(value), true));
            has_explicit = true;
        }
        table.class_name = raw_value.clone();
        table.look = if has_explicit {
            TableLook {
                first_row: Some(explicit.first_row.unwrap_or(false)),
                last_row: Some(explicit.last_row.unwrap_or(false)),
                first_column: Some(explicit.first_column.unwrap_or(false)),
                last_column: Some(explicit.last_column.unwrap_or(false)),
                no_h_band: Some(explicit.no_h_band.unwrap_or(false)),
                no_v_band: Some(explicit.no_v_band.unwrap_or(false)),
            }
        } else {
            raw_value
                .as_deref()
                .and_then(|value| u16::from_str_radix(value.trim(), 16).ok())
                .map(TableLook::from_mask)
                // An empty or malformed element behaves like an omitted one.
                .unwrap_or_default()
        };
    }

    pub(super) fn parse_table_row_properties(
        &self,
        reader: &mut dyn XmlEventSource,
        row: &mut TableRow,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => match e.name().as_ref() {
                    b"w:cnfStyle" => {
                        row.class_name = Self::read_val_attribute(e.attributes());
                        self.skip_element(reader, b"w:cnfStyle")?;
                    }
                    b"w:trHeight" => {
                        let (height, rule) = Self::parse_table_row_height(e.attributes());
                        row.height = height;
                        row.height_rule = rule;
                        self.skip_element(reader, b"w:trHeight")?;
                    }
                    b"w:tblHeader" => {
                        let value = Self::read_val_attribute(e.attributes());
                        row.is_header = parse_on_off(value, true);
                        self.skip_element(reader, b"w:tblHeader")?;
                    }
                    b"w:cantSplit" => {
                        let value = Self::read_val_attribute(e.attributes());
                        row.cant_split = parse_on_off(value, true);
                        self.skip_element(reader, b"w:cantSplit")?;
                    }
                    b"w:hidden" => {
                        row.style.insert(
                            "hidden".to_string(),
                            parse_on_off(Self::read_val_attribute(e.attributes()), true)
                                .to_string(),
                        );
                        self.skip_element(reader, b"w:hidden")?;
                    }
                    b"w:gridBefore" => {
                        row.grid_before = Self::read_int_attribute(e.attributes());
                        self.skip_element(reader, b"w:gridBefore")?;
                    }
                    b"w:gridAfter" => {
                        row.grid_after = Self::read_int_attribute(e.attributes());
                        self.skip_element(reader, b"w:gridAfter")?;
                    }
                    b"w:wBefore" => {
                        row.width_before = Self::parse_table_width(e.attributes());
                        self.skip_element(reader, b"w:wBefore")?;
                    }
                    b"w:wAfter" => {
                        row.width_after = Self::parse_table_width(e.attributes());
                        self.skip_element(reader, b"w:wAfter")?;
                    }
                    b"w:tblCellSpacing" => {
                        row.cell_spacing = Self::table_cell_spacing_value(e.attributes());
                        self.skip_element(reader, b"w:tblCellSpacing")?;
                    }
                    b"w:ins" | b"w:del" => {
                        let kind = Self::revision_kind(e);
                        let mut mark = Self::build_revision_mark(e, kind);
                        if mark.is_deletion() {
                            mark.insertion = row.revision.take().and_then(|previous| {
                                if previous.is_deletion() {
                                    previous.insertion
                                } else {
                                    Some(previous)
                                }
                            });
                            row.revision = Some(Box::new(mark));
                        } else if let Some(deletion) =
                            row.revision.as_mut().filter(|mark| mark.is_deletion())
                        {
                            deletion.insertion = Some(Box::new(mark));
                        } else {
                            row.revision = Some(Box::new(mark));
                        }
                        self.skip_element(reader, e.name().as_ref())?;
                    }
                    _ => self.skip_element(reader, e.name().as_ref())?,
                },
                Ok(Event::Empty(ref e)) => match e.name().as_ref() {
                    b"w:cnfStyle" => {
                        row.class_name = Self::read_val_attribute(e.attributes());
                    }
                    b"w:trHeight" => {
                        let (height, rule) = Self::parse_table_row_height(e.attributes());
                        row.height = height;
                        row.height_rule = rule;
                    }
                    b"w:tblHeader" => {
                        let value = Self::read_val_attribute(e.attributes());
                        row.is_header = parse_on_off(value, true);
                    }
                    b"w:cantSplit" => {
                        let value = Self::read_val_attribute(e.attributes());
                        row.cant_split = parse_on_off(value, true);
                    }
                    b"w:hidden" => {
                        row.style.insert(
                            "hidden".to_string(),
                            parse_on_off(Self::read_val_attribute(e.attributes()), true)
                                .to_string(),
                        );
                    }
                    b"w:gridBefore" => {
                        row.grid_before = Self::read_int_attribute(e.attributes());
                    }
                    b"w:gridAfter" => {
                        row.grid_after = Self::read_int_attribute(e.attributes());
                    }
                    b"w:wBefore" => {
                        row.width_before = Self::parse_table_width(e.attributes());
                    }
                    b"w:wAfter" => {
                        row.width_after = Self::parse_table_width(e.attributes());
                    }
                    b"w:tblCellSpacing" => {
                        row.cell_spacing = Self::table_cell_spacing_value(e.attributes());
                    }
                    b"w:ins" | b"w:del" => {
                        let kind = Self::revision_kind(e);
                        let mut mark = Self::build_revision_mark(e, kind);
                        if mark.is_deletion() {
                            mark.insertion = row.revision.take().and_then(|previous| {
                                if previous.is_deletion() {
                                    previous.insertion
                                } else {
                                    Some(previous)
                                }
                            });
                            row.revision = Some(Box::new(mark));
                        } else if let Some(deletion) =
                            row.revision.as_mut().filter(|mark| mark.is_deletion())
                        {
                            deletion.insertion = Some(Box::new(mark));
                        } else {
                            row.revision = Some(Box::new(mark));
                        }
                    }
                    _ => {}
                },
                Ok(Event::End(ref e)) => {
                    if e.name().as_ref() == b"w:trPr" {
                        break;
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("XML parsing error: {}", err).into()),
                _ => {}
            }
        }

        Ok(())
    }

    /// Parses row-scoped table property exceptions (`w:tblPrEx`). Word emits
    /// this container before `w:trPr` when a table style or conditional region
    /// supplies properties for a particular row. Only properties with a
    /// complete row/rendering contract should be promoted here; unsupported
    /// exception children remain preserved in the raw package XML.
    pub(super) fn parse_table_property_exceptions(
        &self,
        reader: &mut dyn XmlEventSource,
        row: &mut TableRow,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => match e.name().as_ref() {
                    b"w:tblCellSpacing" => {
                        row.cell_spacing_exception = Self::table_cell_spacing_value(e.attributes());
                        self.skip_element(reader, b"w:tblCellSpacing")?;
                    }
                    b"w:tblCellMar" => {
                        let margins = Self::parse_cell_margins(reader, b"w:tblCellMar")?;
                        if !margins.is_empty() {
                            row.cell_margins = Some(margins);
                        }
                    }
                    b"w:shd" => {
                        if let Some(color) = Self::read_color_attribute(e.attributes(), b"w:fill") {
                            row.style.insert("background-color".to_string(), color);
                        }
                        self.skip_element(reader, b"w:shd")?;
                    }
                    b"w:tblBorders" => {
                        let borders = Self::parse_table_borders(reader)?;
                        if !borders.is_empty() {
                            row.borders = Some(Box::new(borders));
                        }
                    }
                    _ => self.skip_element(reader, e.name().as_ref())?,
                },
                Ok(Event::Empty(ref e)) => match e.name().as_ref() {
                    b"w:tblCellSpacing" => {
                        row.cell_spacing_exception = Self::table_cell_spacing_value(e.attributes());
                    }
                    b"w:shd" => {
                        if let Some(color) = Self::read_color_attribute(e.attributes(), b"w:fill") {
                            row.style.insert("background-color".to_string(), color);
                        }
                    }
                    _ => {}
                },
                Ok(Event::End(ref e)) if e.name().as_ref() == b"w:tblPrEx" => break,
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("XML parsing error: {}", err).into()),
                _ => {}
            }
        }

        Ok(())
    }

    pub(super) fn parse_table_cell_properties(
        &self,
        reader: &mut dyn XmlEventSource,
        cell: &mut TableCell,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => match e.name().as_ref() {
                    b"w:gridSpan" => {
                        cell.span =
                            Self::read_int_attribute(e.attributes()).map(|span| span.max(1));
                        self.skip_element(reader, b"w:gridSpan")?;
                    }
                    b"w:vMerge" => {
                        let value = Self::read_val_attribute(e.attributes())
                            .unwrap_or_else(|| "continue".to_string())
                            .to_ascii_lowercase();
                        cell.vertical_merge = Some(value);
                        self.skip_element(reader, b"w:vMerge")?;
                    }
                    b"w:hMerge" => {
                        let value = Self::read_val_attribute(e.attributes())
                            .unwrap_or_else(|| "continue".to_string())
                            .to_ascii_lowercase();
                        cell.style.insert("horizontal-merge".to_string(), value);
                        self.skip_element(reader, b"w:hMerge")?;
                    }
                    b"w:cnfStyle" => {
                        cell.class_name = Self::read_val_attribute(e.attributes());
                        self.skip_element(reader, b"w:cnfStyle")?;
                    }
                    b"w:tcW" => {
                        if let Some(width) = Self::parse_table_width(e.attributes()) {
                            cell.style.insert("width".to_string(), width);
                        }
                        self.skip_element(reader, b"w:tcW")?;
                    }
                    b"w:shd" => {
                        if let Some(color) = Self::read_color_attribute(e.attributes(), b"w:fill") {
                            cell.style.insert("background-color".to_string(), color);
                        }
                        self.skip_element(reader, b"w:shd")?;
                    }
                    b"w:tcBorders" => {
                        self.parse_table_cell_borders(reader, cell)?;
                    }
                    b"w:tcMar" => {
                        let margins = Self::parse_cell_margins(reader, b"w:tcMar")?;
                        if !margins.is_empty() {
                            cell.margins = Some(margins);
                        }
                    }
                    b"w:hideMark" => {
                        cell.style.insert(
                            "hide-mark".to_string(),
                            parse_on_off(Self::read_val_attribute(e.attributes()), true)
                                .to_string(),
                        );
                        self.skip_element(reader, b"w:hideMark")?;
                    }
                    b"w:noWrap" => {
                        cell.style.insert(
                            "no-wrap".to_string(),
                            parse_on_off(Self::read_val_attribute(e.attributes()), true)
                                .to_string(),
                        );
                        self.skip_element(reader, b"w:noWrap")?;
                    }
                    b"w:tcFitText" => {
                        cell.style.insert(
                            "fit-text".to_string(),
                            parse_on_off(Self::read_val_attribute(e.attributes()), true)
                                .to_string(),
                        );
                        self.skip_element(reader, b"w:tcFitText")?;
                    }
                    b"w:textDirection" => {
                        if let Some(direction) = Self::read_text_direction(e.attributes()) {
                            cell.text_direction = Some(direction);
                        }
                        self.skip_element(reader, b"w:textDirection")?;
                    }
                    b"w:vAlign" => {
                        if let Some(token) = Self::read_val_attribute(e.attributes()) {
                            let token = token.trim().to_ascii_lowercase();
                            if !token.is_empty() {
                                cell.vertical_align = Some(token);
                            }
                        }
                        self.skip_element(reader, b"w:vAlign")?;
                    }
                    _ => self.skip_element(reader, e.name().as_ref())?,
                },
                Ok(Event::Empty(ref e)) => match e.name().as_ref() {
                    b"w:gridSpan" => {
                        cell.span =
                            Self::read_int_attribute(e.attributes()).map(|span| span.max(1));
                    }
                    b"w:vMerge" => {
                        let value = Self::read_val_attribute(e.attributes())
                            .unwrap_or_else(|| "continue".to_string())
                            .to_ascii_lowercase();
                        cell.vertical_merge = Some(value);
                    }
                    b"w:hMerge" => {
                        let value = Self::read_val_attribute(e.attributes())
                            .unwrap_or_else(|| "continue".to_string())
                            .to_ascii_lowercase();
                        cell.style.insert("horizontal-merge".to_string(), value);
                    }
                    b"w:cnfStyle" => {
                        cell.class_name = Self::read_val_attribute(e.attributes());
                    }
                    b"w:tcW" => {
                        if let Some(width) = Self::parse_table_width(e.attributes()) {
                            cell.style.insert("width".to_string(), width);
                        }
                    }
                    b"w:shd" => {
                        if let Some(color) = Self::read_color_attribute(e.attributes(), b"w:fill") {
                            cell.style.insert("background-color".to_string(), color);
                        }
                    }
                    b"w:tcBorders" => {}
                    b"w:hideMark" => {
                        cell.style.insert(
                            "hide-mark".to_string(),
                            parse_on_off(Self::read_val_attribute(e.attributes()), true)
                                .to_string(),
                        );
                    }
                    b"w:noWrap" => {
                        cell.style.insert(
                            "no-wrap".to_string(),
                            parse_on_off(Self::read_val_attribute(e.attributes()), true)
                                .to_string(),
                        );
                    }
                    b"w:tcFitText" => {
                        cell.style.insert(
                            "fit-text".to_string(),
                            parse_on_off(Self::read_val_attribute(e.attributes()), true)
                                .to_string(),
                        );
                    }
                    b"w:textDirection" => {
                        if let Some(direction) = Self::read_text_direction(e.attributes()) {
                            cell.text_direction = Some(direction);
                        }
                    }
                    b"w:vAlign" => {
                        if let Some(token) = Self::read_val_attribute(e.attributes()) {
                            let token = token.trim().to_ascii_lowercase();
                            if !token.is_empty() {
                                cell.vertical_align = Some(token);
                            }
                        }
                    }
                    _ => {}
                },
                Ok(Event::End(ref e)) => {
                    if e.name().as_ref() == b"w:tcPr" {
                        break;
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("XML parsing error: {}", err).into()),
                _ => {}
            }
        }

        Ok(())
    }

    /// Converts the legacy `w:hMerge` cell sequence into the same `gridSpan`
    /// representation used by modern Word documents. Keeping one logical cell
    /// lets layout, selection, resizing, and editing share their existing span
    /// contract instead of teaching each consumer about a second merge model.
    pub(super) fn normalize_legacy_horizontal_merges(row: &mut TableRow) {
        let mut normalized: Vec<TableCell> = Vec::with_capacity(row.cells.len());
        let mut active_restart: Option<usize> = None;

        for mut cell in std::mem::take(&mut row.cells) {
            let merge = cell.style.remove("horizontal-merge");
            match merge.as_deref() {
                Some("restart") => {
                    active_restart = Some(normalized.len());
                    normalized.push(cell);
                }
                Some("continue") => {
                    if let Some(restart_index) = active_restart {
                        // A continuation can itself cover multiple grid units.
                        // Add that complete width to the restart cell, matching
                        // the equivalent modern `w:gridSpan` value.
                        let continued_span = cell.span.unwrap_or(1).max(1);
                        let restart = &mut normalized[restart_index];
                        let restart_span = restart.span.unwrap_or(1).max(1);
                        restart.span = Some(restart_span.saturating_add(continued_span));
                        if Self::legacy_merge_blocks_have_content(&cell.blocks) {
                            // Word's editing UI can leave meaningful content in
                            // a continuation cell in old documents. Preserve it
                            // in document order, but discard the restart cell's
                            // mandatory empty paragraph when it is only a marker.
                            if !Self::legacy_merge_blocks_have_content(&restart.blocks) {
                                restart.blocks.clear();
                            }
                            restart.blocks.append(&mut cell.blocks);
                        }
                    } else {
                        // Do not discard malformed orphan continuations: there
                        // is no preceding restart whose geometry can own them.
                        normalized.push(cell);
                    }
                }
                _ => {
                    active_restart = None;
                    normalized.push(cell);
                }
            }
        }

        row.cells = normalized;
    }

    fn legacy_merge_blocks_have_content(blocks: &[BodyElement]) -> bool {
        // Multiple paragraphs are meaningful even when currently empty: they
        // represent explicit paragraph breaks retained by the old merge.
        blocks.len() > 1
            || blocks.iter().any(|block| match block {
                BodyElement::Table(_) | BodyElement::Math(_) => true,
                BodyElement::Paragraph(paragraph) => paragraph.runs.iter().any(|run| {
                    !run.text.is_empty()
                        || run.image.is_some()
                        || run.text_box.is_some()
                        || run.math.is_some()
                        || run.field.is_some()
                        || !run.comment_reference_ids.is_empty()
                        || !run.footnote_reference_ids.is_empty()
                        || !run.endnote_reference_ids.is_empty()
                        || run.has_footnote_ref
                        || run.has_endnote_ref
                }),
            })
    }

    fn parse_table_cell_borders(
        &self,
        reader: &mut dyn XmlEventSource,
        cell: &mut TableCell,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => match e.name().as_ref() {
                    b"w:top" | b"w:right" | b"w:bottom" | b"w:left" | b"w:start" | b"w:end"
                    | b"w:tl2br" | b"w:tr2bl" => {
                        Self::apply_table_cell_border_style(
                            e.name().as_ref(),
                            e.attributes(),
                            cell,
                        );
                        self.skip_element(reader, e.name().as_ref())?;
                    }
                    _ => self.skip_element(reader, e.name().as_ref())?,
                },
                Ok(Event::Empty(ref e)) => match e.name().as_ref() {
                    b"w:top" | b"w:right" | b"w:bottom" | b"w:left" | b"w:start" | b"w:end"
                    | b"w:tl2br" | b"w:tr2bl" => {
                        Self::apply_table_cell_border_style(
                            e.name().as_ref(),
                            e.attributes(),
                            cell,
                        );
                    }
                    _ => {}
                },
                Ok(Event::End(ref e)) => {
                    if e.name().as_ref() == b"w:tcBorders" {
                        break;
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("XML parsing error: {}", err).into()),
                _ => {}
            }
        }

        Ok(())
    }

    /// Parses a `w:tblBorders` subtree into [`TableBorders`]. Each child edge
    /// (`w:top`/`w:bottom`/`w:left`/`w:right`/`w:insideH`/`w:insideV`) is a
    /// self-closing element with the same `w:val`/`w:sz`/`w:space`/`w:color`
    /// attributes as a paragraph border edge, so we reuse `build_border_edge`.
    fn parse_table_borders(
        reader: &mut dyn XmlEventSource,
    ) -> Result<crate::document::TableBorders, Box<dyn std::error::Error>> {
        use crate::document::TableBorders;
        let mut borders = TableBorders::default();
        let mut buf = Vec::new();

        let mut assign = |name: &[u8], attrs: Attributes<'_>| {
            let edge = Self::build_border_edge(attrs);
            match name {
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
        };

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::End(ref e)) if e.name().as_ref() == b"w:tblBorders" => break,
                Ok(Event::Empty(ref e)) => {
                    let name = e.name();
                    assign(name.as_ref(), e.attributes());
                }
                Ok(Event::Start(ref e)) => {
                    let owned = e.name();
                    let name = owned.as_ref().to_vec();
                    assign(&name, e.attributes());
                    // Edge elements are normally self-closing; tolerate a
                    // Start by consuming its (empty) body.
                    let mut depth = 1usize;
                    let mut inner = Vec::new();
                    loop {
                        inner.clear();
                        match reader.read_event_into(&mut inner) {
                            Ok(Event::Start(ref s)) if s.name().as_ref() == name.as_slice() => {
                                depth += 1
                            }
                            Ok(Event::End(ref s)) if s.name().as_ref() == name.as_slice() => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Ok(Event::Eof) => break,
                            _ => {}
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("XML parsing error: {}", err).into()),
                _ => {}
            }
        }

        Ok(borders)
    }

    /// Converts a single `w:tblCellMar`/`w:tcMar` edge element's attributes into
    /// a margin measured in points. `w:w` is in twentieths of a point (dxa);
    /// `w:type="nil"` forces a zero margin regardless of `w:w`. Percentage
    /// margins are uncommon and currently ignored (treated as absent).
    fn cell_margin_value(attributes: Attributes<'_>) -> Option<f32> {
        let mut width: Option<f32> = None;
        let mut width_type: Option<String> = None;
        for attr in attributes.flatten() {
            match attr.key.as_ref() {
                b"w:w" => {
                    width = String::from_utf8_lossy(&attr.value)
                        .trim()
                        .parse::<f32>()
                        .ok();
                }
                b"w:type" => {
                    width_type = Some(String::from_utf8_lossy(&attr.value).trim().to_string());
                }
                _ => {}
            }
        }
        match width_type.as_deref() {
            Some("nil") => Some(0.0),
            Some("pct") => None,
            _ => width.map(|value| value / 20.0),
        }
    }

    /// Converts `w:tblCellSpacing` to points. The element shares the common
    /// table-width attribute shape, but ECMA-376 explicitly ignores `pct` and
    /// `auto` here. An omitted type is DXA; `nil` is an explicit zero.
    fn table_cell_spacing_value(attributes: Attributes<'_>) -> Option<f32> {
        let mut width: Option<f32> = None;
        let mut width_type: Option<String> = None;
        for attr in attributes.flatten() {
            match attr.key.as_ref() {
                b"w:w" => {
                    width = String::from_utf8_lossy(&attr.value)
                        .trim()
                        .parse::<f32>()
                        .ok();
                }
                b"w:type" => {
                    width_type = Some(
                        String::from_utf8_lossy(&attr.value)
                            .trim()
                            .to_ascii_lowercase(),
                    );
                }
                _ => {}
            }
        }

        match width_type.as_deref() {
            Some("nil") => Some(0.0),
            Some("pct" | "auto") => None,
            Some("dxa") | None => width.map(|value| (value / 20.0).max(0.0)),
            Some(_) => None,
        }
    }

    /// Parses a `w:tblCellMar` (table) or `w:tcMar` (cell) subtree into
    /// [`TableCellMargins`]. The four edges may use either the legacy
    /// `w:left`/`w:right` or the newer `w:start`/`w:end` element names.
    fn parse_cell_margins(
        reader: &mut dyn XmlEventSource,
        end_tag: &[u8],
    ) -> Result<crate::document::TableCellMargins, Box<dyn std::error::Error>> {
        use crate::document::TableCellMargins;
        let mut margins = TableCellMargins::default();
        let mut buf = Vec::new();

        let mut assign = |name: &[u8], attrs: Attributes<'_>| {
            let value = Self::cell_margin_value(attrs);
            match name {
                b"w:top" => margins.top = value,
                b"w:bottom" => margins.bottom = value,
                b"w:left" => margins.left = value,
                b"w:right" => margins.right = value,
                b"w:start" => margins.start = value,
                b"w:end" => margins.end = value,
                _ => {}
            }
        };

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::End(ref e)) if e.name().as_ref() == end_tag => break,
                Ok(Event::Empty(ref e)) => {
                    let name = e.name();
                    assign(name.as_ref(), e.attributes());
                }
                Ok(Event::Start(ref e)) => {
                    let owned = e.name();
                    let name = owned.as_ref().to_vec();
                    assign(&name, e.attributes());
                    // Edge elements are normally self-closing; tolerate a Start
                    // by consuming its (empty) body.
                    let mut depth = 1usize;
                    let mut inner = Vec::new();
                    loop {
                        inner.clear();
                        match reader.read_event_into(&mut inner) {
                            Ok(Event::Start(ref s)) if s.name().as_ref() == name.as_slice() => {
                                depth += 1
                            }
                            Ok(Event::End(ref s)) if s.name().as_ref() == name.as_slice() => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Ok(Event::Eof) => break,
                            _ => {}
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("XML parsing error: {}", err).into()),
                _ => {}
            }
        }

        Ok(margins)
    }

    fn apply_table_cell_border_style(
        side: &[u8],
        attributes: Attributes<'_>,
        cell: &mut TableCell,
    ) {
        let (style_key, width_key, color_key) = match side {
            b"w:top" => ("border-top-style", "border-top-width", "border-top-color"),
            b"w:right" => (
                "border-right-style",
                "border-right-width",
                "border-right-color",
            ),
            b"w:bottom" => (
                "border-bottom-style",
                "border-bottom-width",
                "border-bottom-color",
            ),
            b"w:left" => (
                "border-left-style",
                "border-left-width",
                "border-left-color",
            ),
            b"w:start" => (
                "border-start-style",
                "border-start-width",
                "border-start-color",
            ),
            b"w:end" => ("border-end-style", "border-end-width", "border-end-color"),
            b"w:tl2br" => (
                "border-tl2br-style",
                "border-tl2br-width",
                "border-tl2br-color",
            ),
            b"w:tr2bl" => (
                "border-tr2bl-style",
                "border-tr2bl-width",
                "border-tr2bl-color",
            ),
            _ => return,
        };

        // OOXML stores the border kind in `w:val`, the thickness in `w:sz`
        // (eighths of a point), and the color in `w:color` (hex or "auto").
        // Keep the raw style token (e.g. "single", "double", "dashed") so that
        // the renderer can branch on it; only `nil`/`none` are normalized to
        // the CSS keyword `none` for parity with the rest of the pipeline.
        let mut raw_style: Option<String> = None;
        let mut size_eighths: Option<f32> = None;
        let mut color: Option<String> = None;
        for attr in attributes.flatten() {
            match attr.key.as_ref() {
                b"w:val" => {
                    raw_style = Some(String::from_utf8_lossy(&attr.value).to_string());
                }
                b"w:sz" => {
                    if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                        size_eighths = Some(value);
                    }
                }
                b"w:color" => {
                    let value = String::from_utf8_lossy(&attr.value).to_string();
                    if !value.eq_ignore_ascii_case("auto") {
                        color = Some(if value.starts_with('#') {
                            value
                        } else if value.len() == 6 {
                            format!("#{value}")
                        } else {
                            value
                        });
                    }
                }
                _ => {}
            }
        }

        let normalized_style = raw_style
            .unwrap_or_else(|| "single".to_string())
            .to_ascii_lowercase();
        let style = if matches!(normalized_style.as_str(), "nil" | "none") {
            "none".to_string()
        } else {
            normalized_style
        };
        cell.style.insert(style_key.to_string(), style);

        if let Some(eighths) = size_eighths {
            // Eighths of a point → points → CSS "pt" so `parse_length_to_px`
            // can consume the string later without a dedicated unit.
            let points = eighths / 8.0;
            cell.style
                .insert(width_key.to_string(), format!("{points:.4}pt"));
        }

        if let Some(color) = color {
            cell.style.insert(color_key.to_string(), color);
        }
    }

    fn parse_table_width(attributes: Attributes<'_>) -> Option<String> {
        let mut width: Option<f32> = None;
        let mut width_type: Option<String> = None;

        for attr in attributes.flatten() {
            match attr.key.as_ref() {
                b"w:w" => {
                    if let Ok(val) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                        width = Some(val);
                    }
                }
                b"w:type" => {
                    width_type = Some(String::from_utf8_lossy(&attr.value).to_string());
                }
                _ => {}
            }
        }

        match (width, width_type.as_deref()) {
            (Some(val), Some("pct")) => Some(format!("{:.2}%", val / 50.0)),
            (Some(val), Some("dxa")) | (Some(val), None) => Some(format!("{:.2}pt", val / 20.0)),
            _ => None,
        }
    }

    /// Parses `w:tblInd` independently from `w:tblW`. Both use the common
    /// table-width attribute shape, but indentation only accepts absolute DXA
    /// (or explicit nil/zero); percentage and automatic values are ignored by
    /// Word. Keeping the rules separate prevents relative table widths from
    /// becoming physical canvas offsets.
    fn parse_table_indent(attributes: Attributes<'_>) -> (Option<String>, Option<String>) {
        let mut width: Option<f32> = None;
        let mut width_type: Option<String> = None;
        for attr in attributes.flatten() {
            match attr.key.as_ref() {
                b"w:w" => {
                    width = String::from_utf8_lossy(&attr.value)
                        .trim()
                        .parse::<f32>()
                        .ok();
                }
                b"w:type" => {
                    width_type = Some(
                        String::from_utf8_lossy(&attr.value)
                            .trim()
                            .to_ascii_lowercase(),
                    );
                }
                _ => {}
            }
        }

        let ignored_type = width_type
            .as_deref()
            .filter(|kind| matches!(*kind, "pct" | "auto"))
            .map(str::to_owned);
        let indent = match width_type.as_deref() {
            Some("nil") => Some("0pt".to_string()),
            Some("dxa") | None => width.map(|value| format!("{:.2}pt", value / 20.0)),
            Some("pct" | "auto") | Some(_) => None,
        };
        (indent, ignored_type)
    }

    /// Records a `w:tblpPr` read before `w:tblStyle`, against the schema's
    /// order. Word for Mac drops such a position and lays the table out
    /// inline; the table keeps its position for saving and for Windows.
    fn mark_position_before_style(table: &mut Table) {
        if table.style.contains_key("table-position") {
            table.style.insert(
                "table-position-before-style".to_string(),
                "true".to_string(),
            );
        }
    }

    /// Preserves floating-table positioning metadata from `w:tblpPr`.
    /// Horizontal and vertical placement are consumed by the canvas renderer;
    /// text distances remain model metadata so future paragraph wrapping work
    /// does not need to reparse the raw package.
    fn parse_table_position_properties(table: &mut Table, attributes: Attributes<'_>) {
        table
            .style
            .insert("table-position".to_string(), "floating".to_string());

        for attr in attributes.flatten() {
            let value = String::from_utf8_lossy(&attr.value).trim().to_string();
            let token = value.to_ascii_lowercase();
            let style_value = match attr.key.as_ref() {
                b"w:horzAnchor" if matches!(token.as_str(), "margin" | "page" | "text") => {
                    Some(("table-position-horizontal-anchor", token))
                }
                b"w:vertAnchor" if matches!(token.as_str(), "margin" | "page" | "text") => {
                    Some(("table-position-vertical-anchor", token))
                }
                b"w:tblpXSpec"
                    if matches!(
                        token.as_str(),
                        "center" | "inside" | "left" | "outside" | "right"
                    ) =>
                {
                    Some(("table-position-x-align", token))
                }
                b"w:tblpYSpec"
                    if matches!(
                        token.as_str(),
                        "bottom" | "center" | "inline" | "inside" | "outside" | "top"
                    ) =>
                {
                    Some(("table-position-y-align", token))
                }
                b"w:tblpX" => {
                    Self::twips_style_value(&value).map(|value| ("table-position-x", value))
                }
                b"w:tblpY" => {
                    Self::twips_style_value(&value).map(|value| ("table-position-y", value))
                }
                b"w:leftFromText" => {
                    Self::twips_style_value(&value).map(|value| ("table-distance-left", value))
                }
                b"w:rightFromText" => {
                    Self::twips_style_value(&value).map(|value| ("table-distance-right", value))
                }
                b"w:topFromText" => {
                    Self::twips_style_value(&value).map(|value| ("table-distance-top", value))
                }
                b"w:bottomFromText" => {
                    Self::twips_style_value(&value).map(|value| ("table-distance-bottom", value))
                }
                _ => None,
            };
            if let Some((key, value)) = style_value {
                table.style.insert(key.to_string(), value);
            }
        }
    }

    fn twips_style_value(value: &str) -> Option<String> {
        value
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
            .map(|value| format!("{:.2}pt", value / 20.0))
    }

    fn parse_grid_column_width(attributes: Attributes<'_>) -> Option<f32> {
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:w" {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                    return Some(value / 20.0);
                }
            }
        }
        None
    }

    /// Reads the `w:type` attribute of `w:tblLayout` into a [`TableLayoutMode`].
    /// Returns `None` when the attribute is missing so an absent/incomplete
    /// element falls back to the autofit default at render time.
    fn parse_table_layout_type(attributes: Attributes<'_>) -> Option<TableLayoutMode> {
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:type" {
                let value = String::from_utf8_lossy(&attr.value);
                return Some(TableLayoutMode::from_type_token(&value));
            }
        }
        None
    }

    fn read_val_attribute(attributes: Attributes<'_>) -> Option<String> {
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                return Some(String::from_utf8_lossy(&attr.value).to_string());
            }
        }
        None
    }

    fn read_int_attribute(attributes: Attributes<'_>) -> Option<u32> {
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<u32>() {
                    return Some(value);
                }
            }
        }
        None
    }

    fn read_color_attribute(attributes: Attributes<'_>, key: &[u8]) -> Option<String> {
        for attr in attributes.flatten() {
            if attr.key.as_ref() == key {
                let value = String::from_utf8_lossy(&attr.value).to_string();
                if value.eq_ignore_ascii_case("auto") {
                    return None;
                }
                if value.starts_with('#') {
                    return Some(value);
                }
                if value.len() == 6 {
                    return Some(format!("#{value}"));
                }
                return Some(value);
            }
        }
        None
    }

    fn read_text_direction(attributes: Attributes<'_>) -> Option<TextDirection> {
        for attr in attributes.flatten() {
            if attr.key.as_ref() == b"w:val" {
                let value = String::from_utf8_lossy(&attr.value);
                if let Some(direction) = TextDirection::try_from_str(value.as_ref()) {
                    return Some(direction);
                }
            }
        }
        None
    }

    fn parse_table_row_height(
        attributes: Attributes<'_>,
    ) -> (Option<f32>, Option<TableRowHeightRule>) {
        let mut height = None;
        let mut rule = None;
        for attr in attributes.flatten() {
            match attr.key.as_ref() {
                b"w:val" => {
                    if let Ok(value) = String::from_utf8_lossy(&attr.value).parse::<f32>() {
                        height = Some(value / 20.0);
                    }
                }
                b"w:hRule" => {
                    let value = String::from_utf8_lossy(&attr.value).to_ascii_lowercase();
                    rule = Some(match value.as_str() {
                        "auto" => TableRowHeightRule::Auto,
                        "atleast" => TableRowHeightRule::AtLeast,
                        "exact" => TableRowHeightRule::Exact,
                        _ => TableRowHeightRule::Exact,
                    });
                }
                _ => {}
            }
        }

        if height.is_some() && rule.is_none() {
            // ISO/IEC 29500 defines the missing hRule default as `auto`, but
            // Microsoft's documented Word implementation deliberately differs:
            // once trHeight is present, an omitted hRule is treated as
            // `atLeast`. Preserve that application-specific behavior so cell
            // content can grow beyond the declared minimum instead of being
            // clipped as if the producer had explicitly requested `exact`.
            rule = Some(TableRowHeightRule::AtLeast);
        }

        (height, rule)
    }

    pub(super) fn skip_element(
        &self,
        reader: &mut dyn XmlEventSource,
        _end_name: &[u8],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut depth = 1;
        let mut buf = Vec::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(_)) => depth += 1,
                Ok(Event::End(_)) => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                Ok(Event::Eof) => break,
                Err(err) => return Err(format!("XML parsing error: {}", err).into()),
                _ => {}
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quick_xml::Reader;

    #[test]
    fn skip_element_counts_all_nested_end_tags() {
        let xml = r#"<w:tblPrChange><w:tblPr><w:tblStyle w:val="old"/></w:tblPr></w:tblPrChange><w:after/>"#;
        let parser = DocumentParser::new();
        let mut reader = Reader::from_str(xml);
        assert!(matches!(reader.read_event(), Ok(Event::Start(_))));

        parser
            .skip_element(&mut reader, b"w:tblPrChange")
            .expect("skip nested change record");

        let event = reader.read_event().expect("event after skipped subtree");
        assert!(matches!(event, Event::Empty(ref e) if e.name().as_ref() == b"w:after"));
    }

    #[test]
    fn parses_table_cell_text_direction() {
        let xml = r#"<w:tcPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:textDirection w:val="tbRl"/>
            </w:tcPr>"#;
        let mut reader = Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut cell = TableCell::default();
        let parser = DocumentParser::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) if e.name().as_ref() == b"w:tcPr" => {
                    parser
                        .parse_table_cell_properties(&mut reader, &mut cell)
                        .unwrap();
                    break;
                }
                Ok(Event::Eof) => panic!("tcPr not found"),
                Ok(_) => {}
                Err(err) => panic!("unexpected error: {err}"),
            }
        }

        assert_eq!(cell.text_direction, Some(TextDirection::TbRl));
    }

    #[test]
    fn parses_hide_mark_on_off_values() {
        fn parse(xml: &str) -> TableCell {
            let mut reader = Reader::from_str(xml);
            let mut buf = Vec::new();
            let mut cell = TableCell::default();
            let parser = DocumentParser::new();
            loop {
                buf.clear();
                match reader.read_event_into(&mut buf) {
                    Ok(Event::Start(ref element)) if element.name().as_ref() == b"w:tcPr" => {
                        parser
                            .parse_table_cell_properties(&mut reader, &mut cell)
                            .expect("parse tcPr");
                        return cell;
                    }
                    Ok(Event::Eof) => panic!("tcPr not found"),
                    Ok(_) => {}
                    Err(error) => panic!("unexpected error: {error}"),
                }
            }
        }

        let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        assert!(parse(&format!(r#"<w:tcPr {ns}><w:hideMark/></w:tcPr>"#))
            .style
            .get("hide-mark")
            .is_some_and(|value| value == "true"));
        assert!(parse(&format!(
            r#"<w:tcPr {ns}><w:hideMark></w:hideMark></w:tcPr>"#
        ))
        .style
        .get("hide-mark")
        .is_some_and(|value| value == "true"));
        assert!(parse(&format!(
            r#"<w:tcPr {ns}><w:hideMark w:val="false"/></w:tcPr>"#
        ))
        .style
        .get("hide-mark")
        .is_none_or(|value| value != "true"));
    }

    #[test]
    fn parses_no_wrap_on_off_values() {
        fn parse(xml: &str) -> TableCell {
            let mut reader = Reader::from_str(xml);
            let mut buf = Vec::new();
            let mut cell = TableCell::default();
            let parser = DocumentParser::new();
            loop {
                buf.clear();
                match reader.read_event_into(&mut buf) {
                    Ok(Event::Start(ref element)) if element.name().as_ref() == b"w:tcPr" => {
                        parser
                            .parse_table_cell_properties(&mut reader, &mut cell)
                            .expect("parse tcPr");
                        return cell;
                    }
                    Ok(Event::Eof) => panic!("tcPr not found"),
                    Ok(_) => {}
                    Err(error) => panic!("unexpected error: {error}"),
                }
            }
        }

        let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        let enabled = parse(&format!(r#"<w:tcPr {ns}><w:noWrap/></w:tcPr>"#));
        let disabled = parse(&format!(
            r#"<w:tcPr {ns}><w:noWrap w:val="false"></w:noWrap></w:tcPr>"#
        ));

        assert_eq!(
            enabled.style.get("no-wrap").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            disabled.style.get("no-wrap").map(String::as_str),
            Some("false")
        );
    }

    #[test]
    fn parses_table_cell_fit_text_on_off_values() {
        // `tcFitText` is an on/off cell property. A bare element enables it,
        // while an explicit false value must survive parsing so the renderer
        // does not condense text inherited from a table style.
        let ns = r#"xmlns:w="urn:w""#;
        let parse = |xml: &str| {
            let mut reader = Reader::from_str(xml);
            reader.config_mut().trim_text(true);
            let parser = DocumentParser::new();
            let mut cell = TableCell::default();
            let mut buf = Vec::new();
            loop {
                match reader.read_event_into(&mut buf).expect("valid XML") {
                    Event::Start(event) if event.name().as_ref() == b"w:tcPr" => {
                        parser
                            .parse_table_cell_properties(&mut reader, &mut cell)
                            .expect("cell properties");
                        break;
                    }
                    Event::Eof => panic!("missing tcPr"),
                    _ => buf.clear(),
                }
            }
            cell
        };

        let enabled = parse(&format!(r#"<w:tcPr {ns}><w:tcFitText/></w:tcPr>"#));
        let disabled = parse(&format!(
            r#"<w:tcPr {ns}><w:tcFitText w:val="false"/></w:tcPr>"#
        ));

        assert_eq!(
            enabled.style.get("fit-text").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            disabled.style.get("fit-text").map(String::as_str),
            Some("false")
        );
    }

    #[test]
    fn parses_legacy_horizontal_merge_values() {
        fn parse(xml: &str) -> TableCell {
            let mut reader = Reader::from_str(xml);
            let mut buf = Vec::new();
            let mut cell = TableCell::default();
            let parser = DocumentParser::new();
            loop {
                buf.clear();
                match reader.read_event_into(&mut buf) {
                    Ok(Event::Start(ref element)) if element.name().as_ref() == b"w:tcPr" => {
                        parser
                            .parse_table_cell_properties(&mut reader, &mut cell)
                            .expect("parse tcPr");
                        return cell;
                    }
                    Ok(Event::Eof) => panic!("tcPr not found"),
                    Ok(_) => {}
                    Err(error) => panic!("unexpected error: {error}"),
                }
            }
        }

        let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        let restart = parse(&format!(
            r#"<w:tcPr {ns}><w:hMerge w:val="restart"/></w:tcPr>"#
        ));
        let implicit_continue = parse(&format!(r#"<w:tcPr {ns}><w:hMerge></w:hMerge></w:tcPr>"#));

        assert_eq!(
            restart.style.get("horizontal-merge").map(String::as_str),
            Some("restart")
        );
        assert_eq!(
            implicit_continue
                .style
                .get("horizontal-merge")
                .map(String::as_str),
            Some("continue")
        );
    }

    #[test]
    fn legacy_horizontal_merge_normalizes_to_grid_span() {
        let merged = |value: &str, span: Option<u32>| TableCell {
            style: [("horizontal-merge".to_string(), value.to_string())]
                .into_iter()
                .collect(),
            span,
            ..Default::default()
        };
        let mut row = TableRow {
            cells: vec![
                TableCell::default(),
                merged("restart", None),
                merged("continue", Some(2)),
                TableCell::default(),
            ],
            ..Default::default()
        };

        DocumentParser::normalize_legacy_horizontal_merges(&mut row);

        assert_eq!(row.cells.len(), 3);
        assert_eq!(row.cells[1].span, Some(3));
        assert!(row
            .cells
            .iter()
            .all(|cell| !cell.style.contains_key("horizontal-merge")));

        // Malformed legacy files sometimes start with a continuation. Without
        // a preceding restart there is no safe cell to absorb it, so preserve
        // its geometry as an ordinary cell rather than dropping content.
        let mut orphan = TableRow {
            cells: vec![merged("continue", None)],
            ..Default::default()
        };
        DocumentParser::normalize_legacy_horizontal_merges(&mut orphan);
        assert_eq!(orphan.cells.len(), 1);
        assert_eq!(orphan.cells[0].span, None);

        let mut content_row = TableRow {
            cells: vec![
                merged("restart", None),
                TableCell {
                    blocks: vec![BodyElement::from_table(Table::default())],
                    style: [("horizontal-merge".to_string(), "continue".to_string())]
                        .into_iter()
                        .collect(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        DocumentParser::normalize_legacy_horizontal_merges(&mut content_row);
        assert_eq!(content_row.cells.len(), 1);
        assert!(matches!(
            content_row.cells[0].blocks.as_slice(),
            [BodyElement::Table(_)]
        ));
    }

    #[test]
    fn grid_span_zero_is_normalized_for_start_and_empty_elements() {
        fn parse(xml: &str) -> TableCell {
            let mut reader = Reader::from_str(xml);
            let mut buf = Vec::new();
            let mut cell = TableCell::default();
            let parser = DocumentParser::new();
            loop {
                buf.clear();
                match reader.read_event_into(&mut buf) {
                    Ok(Event::Start(ref element)) if element.name().as_ref() == b"w:tcPr" => {
                        parser
                            .parse_table_cell_properties(&mut reader, &mut cell)
                            .expect("parse tcPr");
                        return cell;
                    }
                    Ok(Event::Eof) => panic!("tcPr not found"),
                    Ok(_) => {}
                    Err(error) => panic!("unexpected error: {error}"),
                }
            }
        }

        let empty = parse(r#"<w:tcPr xmlns:w="urn:w"><w:gridSpan w:val="0"/></w:tcPr>"#);
        let expanded =
            parse(r#"<w:tcPr xmlns:w="urn:w"><w:gridSpan w:val="0"></w:gridSpan></w:tcPr>"#);
        assert_eq!(empty.span, Some(1));
        assert_eq!(expanded.span, Some(1));
    }

    #[test]
    fn parses_row_height_without_rule_as_word_at_least_default() {
        let xml = r#"<w:trPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:trHeight w:val="1134"/>
            </w:trPr>"#;
        let mut reader = Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut row = TableRow::default();
        let parser = DocumentParser::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) if e.name().as_ref() == b"w:trPr" => {
                    parser
                        .parse_table_row_properties(&mut reader, &mut row)
                        .unwrap();
                    break;
                }
                Ok(Event::Eof) => panic!("trPr not found"),
                Ok(_) => {}
                Err(err) => panic!("unexpected error: {err}"),
            }
        }

        assert_eq!(row.height, Some(1134.0 / 20.0));
        assert_eq!(row.height_rule, Some(TableRowHeightRule::AtLeast));
    }

    fn parse_row_pr(xml: &str) -> TableRow {
        let mut reader = Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut row = TableRow::default();
        let parser = DocumentParser::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) if e.name().as_ref() == b"w:trPr" => {
                    parser
                        .parse_table_row_properties(&mut reader, &mut row)
                        .unwrap();
                    break;
                }
                Ok(Event::Eof) => panic!("trPr not found"),
                Ok(_) => {}
                Err(err) => panic!("unexpected error: {err}"),
            }
        }

        row
    }

    #[test]
    fn a_tracked_row_insertion_or_deletion_is_read() {
        let inserted = parse_row_pr(
            r#"<w:trPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:ins w:id="7" w:author="Ann" w:date="2026-01-02T03:04:05Z"/>
            </w:trPr>"#,
        );
        let mark = inserted.revision.expect("the row is an insertion");
        assert_eq!(mark.kind, RevisionKind::Inserted);
        assert_eq!(mark.id.as_deref(), Some("7"));
        assert_eq!(mark.author.as_deref(), Some("Ann"));
        let deleted = parse_row_pr(
            r#"<w:trPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:del w:id="8" w:author="Bob"></w:del>
            </w:trPr>"#,
        );
        assert_eq!(deleted.revision.unwrap().kind, RevisionKind::Deleted);
        assert!(parse_row_pr("<w:trPr></w:trPr>").revision.is_none());
    }

    #[test]
    fn row_insertion_and_deletion_survive_in_either_xml_order() {
        for paired in [
            r#"<w:ins w:id="7" w:author="Ann"/><w:del w:id="8" w:author="Bob"/>"#,
            r#"<w:del w:id="8" w:author="Bob"></w:del><w:ins w:id="7" w:author="Ann"></w:ins>"#,
        ] {
            let row = parse_row_pr(&format!("<w:trPr>{paired}</w:trPr>"));
            let deletion = row.revision.unwrap();
            assert_eq!(deletion.kind, RevisionKind::Deleted);
            assert_eq!(deletion.author.as_deref(), Some("Bob"));
            let insertion = deletion.insertion.unwrap();
            assert_eq!(insertion.kind, RevisionKind::Inserted);
            assert_eq!(insertion.id.as_deref(), Some("7"));
            assert_eq!(insertion.author.as_deref(), Some("Ann"));
        }
    }

    #[test]
    fn parses_cant_split_row_property() {
        // Present without `w:val` defaults to true (ECMA-376 §17.4.6).
        let present = parse_row_pr(
            r#"<w:trPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:cantSplit/>
            </w:trPr>"#,
        );
        assert!(present.cant_split);

        // Explicit `w:val="false"` disables it.
        let disabled = parse_row_pr(
            r#"<w:trPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:cantSplit w:val="false"/>
            </w:trPr>"#,
        );
        assert!(!disabled.cant_split);

        // Absent leaves the default (false = splittable).
        let absent = parse_row_pr(
            r#"<w:trPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:trHeight w:val="567"/>
            </w:trPr>"#,
        );
        assert!(!absent.cant_split);
    }

    #[test]
    fn parses_repeating_table_header_on_off_values() {
        // `tblHeader` is an on/off property. Word normally emits the compact
        // empty form, so absence of w:val must enable repetition; explicit
        // false must disable it and terminate the contiguous header block.
        let enabled = parse_row_pr(r#"<w:trPr xmlns:w="urn:w"><w:tblHeader/></w:trPr>"#);
        let expanded =
            parse_row_pr(r#"<w:trPr xmlns:w="urn:w"><w:tblHeader></w:tblHeader></w:trPr>"#);
        let disabled =
            parse_row_pr(r#"<w:trPr xmlns:w="urn:w"><w:tblHeader w:val="false"/></w:trPr>"#);

        assert!(enabled.is_header);
        assert!(expanded.is_header);
        assert!(!disabled.is_header);
    }

    #[test]
    fn parses_hidden_table_row_on_off_values() {
        // `w:hidden` belongs to trPr and follows the ordinary on/off rules.
        // Preserve explicit false so a direct row property can override an
        // inherited hidden marker once table-style inheritance is expanded.
        let enabled = parse_row_pr(r#"<w:trPr xmlns:w="urn:w"><w:hidden/></w:trPr>"#);
        let expanded = parse_row_pr(r#"<w:trPr xmlns:w="urn:w"><w:hidden></w:hidden></w:trPr>"#);
        let disabled =
            parse_row_pr(r#"<w:trPr xmlns:w="urn:w"><w:hidden w:val="false"/></w:trPr>"#);

        assert_eq!(
            enabled.style.get("hidden").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            expanded.style.get("hidden").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            disabled.style.get("hidden").map(String::as_str),
            Some("false")
        );
    }

    #[test]
    fn parses_preferred_widths_before_and_after_row_cells() {
        // gridBefore/gridAfter describe how many shared grid columns are
        // omitted from this row; wBefore/wAfter carry the preferred total
        // width of those omitted regions. Both values are required to recover
        // geometry when tblGrid itself is missing or incomplete.
        let row = parse_row_pr(
            r#"<w:trPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:gridBefore w:val="1"/>
                <w:wBefore w:w="1440" w:type="dxa"/>
                <w:gridAfter w:val="2"/>
                <w:wAfter w:w="2500" w:type="pct"/>
            </w:trPr>"#,
        );

        assert_eq!(row.grid_before, Some(1));
        assert_eq!(row.width_before.as_deref(), Some("72.00pt"));
        assert_eq!(row.grid_after, Some(2));
        assert_eq!(row.width_after.as_deref(), Some("50.00%"));
    }

    #[test]
    fn parses_table_indent_into_margin_left() {
        let xml = r#"<w:tblPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:tblInd w:w="480" w:type="dxa"/>
            </w:tblPr>"#;
        let mut reader = Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut table = Table::default();
        let parser = DocumentParser::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) if e.name().as_ref() == b"w:tblPr" => {
                    parser
                        .parse_table_properties(&mut reader, &mut table)
                        .unwrap();
                    break;
                }
                Ok(Event::Eof) => panic!("tblPr not found"),
                Ok(_) => {}
                Err(err) => panic!("unexpected error: {err}"),
            }
        }

        assert_eq!(
            table.style.get("margin-left").map(String::as_str),
            Some("24.00pt")
        );
    }

    #[test]
    fn marks_a_floating_position_read_before_the_table_style() {
        let marker = |xml: &str| {
            parse_table_props(xml)
                .style
                .get("table-position-before-style")
                .cloned()
        };
        assert_eq!(
            marker(
                r#"<w:tblPr xmlns:w="urn:w"><w:tblpPr w:tblpY="3600"/><w:tblStyle w:val="TableGrid"/></w:tblPr>"#
            ),
            Some("true".to_string())
        );
        assert_eq!(
            marker(
                r#"<w:tblPr xmlns:w="urn:w"><w:tblStyle w:val="TableGrid"/><w:tblpPr w:tblpY="3600"/></w:tblPr>"#
            ),
            None
        );
    }

    #[test]
    fn parses_floating_table_horizontal_position_and_text_distances() {
        // tblpPr is a leaf in ordinary Word output, but accept the expanded
        // form too because XML producers are free to serialize an empty leaf
        // with separate start/end tags.
        let relative = parse_table_props(
            r#"<w:tblPr xmlns:w="urn:w"><w:tblpPr w:horzAnchor="margin" w:vertAnchor="page" w:tblpXSpec="right" w:tblpY="1440" w:leftFromText="120" w:rightFromText="240"/></w:tblPr>"#,
        );
        assert_eq!(
            relative.style.get("table-position").map(String::as_str),
            Some("floating")
        );
        assert_eq!(
            relative
                .style
                .get("table-position-horizontal-anchor")
                .map(String::as_str),
            Some("margin")
        );
        assert_eq!(
            relative
                .style
                .get("table-position-x-align")
                .map(String::as_str),
            Some("right")
        );
        assert_eq!(
            relative.style.get("table-position-y").map(String::as_str),
            Some("72.00pt")
        );
        assert_eq!(
            relative
                .style
                .get("table-distance-left")
                .map(String::as_str),
            Some("6.00pt")
        );
        assert_eq!(
            relative
                .style
                .get("table-distance-right")
                .map(String::as_str),
            Some("12.00pt")
        );

        let absolute = parse_table_props(
            r#"<w:tblPr xmlns:w="urn:w"><w:tblpPr w:horzAnchor="page" w:tblpX="-360"></w:tblpPr></w:tblPr>"#,
        );
        assert_eq!(
            absolute.style.get("table-position-x").map(String::as_str),
            Some("-18.00pt")
        );
        assert_eq!(
            absolute
                .style
                .get("table-position-horizontal-anchor")
                .map(String::as_str),
            Some("page")
        );
    }

    #[test]
    fn parses_floating_table_overlap_policy() {
        // Word allows floating tables to overlap when tblOverlap is omitted.
        // Preserve the explicit policy so the renderer can move either
        // participant when a "never" frame intersects another table.
        let never = parse_table_props(
            r#"<w:tblPr xmlns:w="urn:w"><w:tblpPr w:horzAnchor="page" w:vertAnchor="page"/><w:tblOverlap w:val="never"/></w:tblPr>"#,
        );
        assert_eq!(
            never.style.get("table-overlap").map(String::as_str),
            Some("never")
        );

        let allowed = parse_table_props(
            r#"<w:tblPr xmlns:w="urn:w"><w:tblpPr w:horzAnchor="page" w:vertAnchor="page"/><w:tblOverlap w:val="overlap"/></w:tblPr>"#,
        );
        assert_eq!(
            allowed.style.get("table-overlap").map(String::as_str),
            Some("overlap")
        );
    }

    #[test]
    fn ignored_table_indent_retains_its_declared_width_type() {
        for kind in ["auto", "pct"] {
            for expanded in [false, true] {
                let ending = if expanded { "></w:tblInd>" } else { "/>" };
                let xml = format!("<w:tblPr xmlns:w=\"urn:w\"><w:tblInd w:w=\"5000\" w:type=\"{kind}\"{ending}</w:tblPr>");
                let table = parse_table_props(&xml);
                assert!(!table.style.contains_key("margin-left"));
                assert_eq!(
                    table
                        .style
                        .get("table-indent-width-type")
                        .map(String::as_str),
                    Some(kind)
                );
            }
        }
    }

    #[test]
    fn table_indent_only_accepts_absolute_or_nil_widths() {
        // tblInd reuses the table-width attribute shape, but ECMA-376 says
        // percentage and automatic values are ignored. A relative percentage
        // must not leak into the style map as though it were a physical canvas
        // offset; `auto` likewise carries no indentation.
        for width_type in ["auto", "pct"] {
            let xml = format!(
                r#"<w:tblPr xmlns:w="urn:w"><w:tblInd w:w="5000" w:type="{width_type}"/></w:tblPr>"#
            );
            assert!(!parse_table_props(&xml).style.contains_key("margin-left"));
        }

        // nil is an explicit zero rather than absence, while omitted type uses
        // DXA just like the common absolute form.
        let nil = parse_table_props(
            r#"<w:tblPr xmlns:w="urn:w"><w:tblInd w:w="720" w:type="nil"/></w:tblPr>"#,
        );
        assert_eq!(
            nil.style.get("margin-left").map(String::as_str),
            Some("0pt")
        );

        let implicit_dxa =
            parse_table_props(r#"<w:tblPr xmlns:w="urn:w"><w:tblInd w:w="720"/></w:tblPr>"#);
        assert_eq!(
            implicit_dxa.style.get("margin-left").map(String::as_str),
            Some("36.00pt")
        );
    }

    #[test]
    fn parses_table_cell_border_styles() {
        let xml = r#"<w:tcPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:tcBorders>
                    <w:top w:val="nil"/>
                    <w:right w:val="single"/>
                    <w:bottom w:val="dashed"/>
                    <w:left w:val="double"/>
                    <w:tl2br w:val="double" w:sz="16" w:color="C00000"/>
                    <w:tr2bl w:val="dashed" w:sz="8" w:color="4472C4"/>
                </w:tcBorders>
            </w:tcPr>"#;
        let mut reader = Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut cell = TableCell::default();
        let parser = DocumentParser::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) if e.name().as_ref() == b"w:tcPr" => {
                    parser
                        .parse_table_cell_properties(&mut reader, &mut cell)
                        .unwrap();
                    break;
                }
                Ok(Event::Eof) => panic!("tcPr not found"),
                Ok(_) => {}
                Err(err) => panic!("unexpected error: {err}"),
            }
        }

        assert_eq!(
            cell.style.get("border-top-style").map(String::as_str),
            Some("none")
        );
        assert_eq!(
            cell.style.get("border-right-style").map(String::as_str),
            Some("single")
        );
        assert_eq!(
            cell.style.get("border-bottom-style").map(String::as_str),
            Some("dashed")
        );
        assert_eq!(
            cell.style.get("border-left-style").map(String::as_str),
            Some("double")
        );
        assert_eq!(
            cell.style.get("border-tl2br-style").map(String::as_str),
            Some("double")
        );
        assert_eq!(
            cell.style.get("border-tl2br-color").map(String::as_str),
            Some("#C00000")
        );
        assert_eq!(
            cell.style.get("border-tr2bl-style").map(String::as_str),
            Some("dashed")
        );
        assert_eq!(
            cell.style.get("border-tr2bl-width").map(String::as_str),
            Some("1.0000pt")
        );
    }

    #[test]
    fn parses_table_cell_border_size_and_color() {
        // `w:sz="24"` = 24 eighths of a point = 3pt, normalized to a "pt" string
        // so the editor's shared length parser can consume it.
        let xml = r#"<w:tcPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:tcBorders>
                    <w:top w:val="single" w:sz="24" w:color="FF0000"/>
                    <w:right w:val="dotted" w:sz="4" w:color="auto"/>
                </w:tcBorders>
            </w:tcPr>"#;
        let mut reader = Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut cell = TableCell::default();
        let parser = DocumentParser::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) if e.name().as_ref() == b"w:tcPr" => {
                    parser
                        .parse_table_cell_properties(&mut reader, &mut cell)
                        .unwrap();
                    break;
                }
                Ok(Event::Eof) => panic!("tcPr not found"),
                Ok(_) => {}
                Err(err) => panic!("unexpected error: {err}"),
            }
        }

        assert_eq!(
            cell.style.get("border-top-style").map(String::as_str),
            Some("single")
        );
        assert_eq!(
            cell.style.get("border-top-width").map(String::as_str),
            Some("3.0000pt")
        );
        assert_eq!(
            cell.style.get("border-top-color").map(String::as_str),
            Some("#FF0000")
        );
        assert_eq!(
            cell.style.get("border-right-width").map(String::as_str),
            Some("0.5000pt")
        );
        // `auto` should not emit a color so downstream fallbacks apply.
        assert!(!cell.style.contains_key("border-right-color"));
    }

    fn parse_table_props(xml: &str) -> Table {
        let mut reader = Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut table = Table::default();
        let parser = DocumentParser::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) if e.name().as_ref() == b"w:tblPr" => {
                    parser
                        .parse_table_properties(&mut reader, &mut table)
                        .unwrap();
                    break;
                }
                Ok(Event::Eof) => panic!("tblPr not found"),
                Ok(_) => {}
                Err(err) => panic!("unexpected error: {err}"),
            }
        }

        table
    }

    #[test]
    fn parses_table_borders_all_edges() {
        let xml = r#"<w:tblPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:tblBorders>
                    <w:top w:val="single" w:sz="4" w:space="0" w:color="FF0000"/>
                    <w:bottom w:val="Double" w:sz="12" w:color="00FF00"/>
                    <w:left w:val="dashed" w:sz="8" w:color="auto"/>
                    <w:right w:val="dotted" w:sz="6"/>
                    <w:insideH w:val="single" w:sz="2" w:color="0000FF"/>
                    <w:insideV w:val="single" w:sz="2" w:color="0000FF"/>
                </w:tblBorders>
            </w:tblPr>"#;
        let table = parse_table_props(xml);
        let borders = table.borders.expect("tblBorders should be parsed");

        let top = borders.top.expect("top");
        assert_eq!(top.val.as_deref(), Some("single"));
        assert_eq!(top.size_eighths, Some(4));
        assert_eq!(top.space, Some(0));
        assert_eq!(top.color.as_deref(), Some("#FF0000"));

        // `w:val` is lowercased so renderers can compare case-insensitively.
        let bottom = borders.bottom.expect("bottom");
        assert_eq!(bottom.val.as_deref(), Some("double"));
        assert_eq!(bottom.size_eighths, Some(12));
        assert_eq!(bottom.color.as_deref(), Some("#00FF00"));

        // `auto` color is dropped so downstream fallbacks apply.
        let left = borders.left.expect("left");
        assert_eq!(left.val.as_deref(), Some("dashed"));
        assert!(left.color.is_none());

        let right = borders.right.expect("right");
        assert_eq!(right.val.as_deref(), Some("dotted"));
        assert_eq!(right.size_eighths, Some(6));
        assert!(right.color.is_none());

        let inside_h = borders.inside_h.expect("insideH");
        assert_eq!(inside_h.val.as_deref(), Some("single"));
        assert_eq!(inside_h.color.as_deref(), Some("#0000FF"));

        let inside_v = borders.inside_v.expect("insideV");
        assert_eq!(inside_v.val.as_deref(), Some("single"));
    }

    #[test]
    fn omits_table_borders_when_absent() {
        let xml = r#"<w:tblPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:tblInd w:w="480" w:type="dxa"/>
            </w:tblPr>"#;
        let table = parse_table_props(xml);
        assert!(table.borders.is_none());
    }

    #[test]
    fn parses_table_layout_fixed() {
        // `<w:tblLayout w:type="fixed"/>` is the common self-closing form.
        let xml = r#"<w:tblPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:tblLayout w:type="fixed"/>
            </w:tblPr>"#;
        let table = parse_table_props(xml);
        assert_eq!(table.table_layout, Some(TableLayoutMode::Fixed));
    }

    #[test]
    fn parses_table_layout_autofit_token() {
        // An explicit `autofit` token resolves to the autofit mode.
        let xml = r#"<w:tblPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:tblLayout w:type="autofit"/>
            </w:tblPr>"#;
        let table = parse_table_props(xml);
        assert_eq!(table.table_layout, Some(TableLayoutMode::Autofit));
    }

    #[test]
    fn table_layout_absent_is_none() {
        // Absent `w:tblLayout` leaves the field `None` so the renderer applies
        // the autofit default itself.
        let xml = r#"<w:tblPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:tblInd w:w="480" w:type="dxa"/>
            </w:tblPr>"#;
        let table = parse_table_props(xml);
        assert!(table.table_layout.is_none());
    }

    #[test]
    fn parses_table_bidi_visual_on_off_values() {
        let enabled = parse_table_props(
            r#"<w:tblPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:bidiVisual/></w:tblPr>"#,
        );
        let disabled = parse_table_props(
            r#"<w:tblPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:bidiVisual w:val="0"/></w:tblPr>"#,
        );

        assert!(
            format!("{enabled:?}").contains("bidi_visual: true"),
            "an empty on/off element enables visual RTL table order"
        );
        assert!(
            format!("{disabled:?}").contains("bidi_visual: false"),
            "an explicit false value must not mirror the table"
        );
    }

    #[test]
    fn parses_table_look_bitmask_and_explicit_overrides() {
        let defaults =
            parse_table_props(r#"<w:tblPr xmlns:w="urn:w"><w:tblLook w:val="0220"/></w:tblPr>"#);
        assert_eq!(defaults.class_name.as_deref(), Some("0220"));
        assert_eq!(defaults.look.first_row, Some(true));
        assert_eq!(defaults.look.no_h_band, Some(true));
        assert_eq!(defaults.look.first_column, Some(false));
        assert_eq!(defaults.look.no_v_band, Some(false));

        // Word ignores w:val once any explicit attribute is present
        // (MS-OI29500 §17.4.55(c)); omitted attributes are then off.
        let overridden = parse_table_props(
            r#"<w:tblPr xmlns:w="urn:w"><w:tblLook w:val="04A0" w:firstRow="0" w:noHBand="false"/></w:tblPr>"#,
        );
        assert_eq!(overridden.look.first_row, Some(false));
        assert_eq!(overridden.look.no_h_band, Some(false));
        assert_eq!(overridden.look.first_column, Some(false));
        assert_eq!(overridden.look.no_v_band, Some(false));

        let flags_only = parse_table_props(
            r#"<w:tblPr xmlns:w="urn:w"><w:tblLook w:firstRow="1" w:noHBand="1" w:lastColumn="1"/></w:tblPr>"#,
        );
        assert_eq!(flags_only.class_name, None);
        assert_eq!(flags_only.look.first_row, Some(true));
        assert_eq!(flags_only.look.no_h_band, Some(true));
        assert_eq!(flags_only.look.last_column, Some(true));
        assert_eq!(flags_only.look.last_row, Some(false));

        let omitted = parse_table_props(r#"<w:tblPr xmlns:w="urn:w"></w:tblPr>"#);
        assert!(omitted.look.is_unspecified());
        assert_eq!(omitted.look.effective_mask(), 0x04A0);
    }

    #[test]
    fn parses_table_cell_margins() {
        // `w:w` is in dxa (twentieths of a point); 108 dxa = 5.4pt. Preserve
        // logical `w:start`/`w:end` edges until table bidi is known; collapsing
        // them to physical left/right here breaks mirrored tables.
        let xml = r#"<w:tblPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:tblCellMar>
                    <w:top w:w="48" w:type="dxa"/>
                    <w:bottom w:w="48" w:type="dxa"/>
                    <w:start w:w="108" w:type="dxa"/>
                    <w:end w:w="0" w:type="nil"/>
                </w:tblCellMar>
            </w:tblPr>"#;
        let table = parse_table_props(xml);
        let margins = table.cell_margins.expect("tblCellMar should be parsed");
        assert_eq!(margins.top, Some(2.4));
        assert_eq!(margins.bottom, Some(2.4));
        assert_eq!(margins.start, Some(5.4));
        assert_eq!(margins.end, Some(0.0));
        assert_eq!(margins.left, None);
        assert_eq!(margins.right, None);
    }

    #[test]
    fn omits_table_cell_margins_when_absent() {
        let xml = r#"<w:tblPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:tblInd w:w="480" w:type="dxa"/>
            </w:tblPr>"#;
        let table = parse_table_props(xml);
        assert!(table.cell_margins.is_none());
    }

    #[test]
    fn parses_table_cell_spacing_in_points_and_ignores_relative_types() {
        // `tblCellSpacing` uses the common table-width syntax, but Word only
        // honors absolute DXA (twentieths of a point) and `nil` values here.
        // Percentage/automatic values must be ignored rather than accidentally
        // becoming a large pixel gap. 144 dxa is the canonical 7.2pt example
        // used by the OOXML documentation.
        let dxa = parse_table_props(
            r#"<w:tblPr xmlns:w="urn:w"><w:tblCellSpacing w:w="144" w:type="dxa"/></w:tblPr>"#,
        );
        assert_eq!(dxa.cell_spacing, Some(7.2));

        let nil = parse_table_props(
            r#"<w:tblPr xmlns:w="urn:w"><w:tblCellSpacing w:w="999" w:type="nil"/></w:tblPr>"#,
        );
        assert_eq!(nil.cell_spacing, Some(0.0));

        for width_type in ["pct", "auto"] {
            let xml = format!(
                r#"<w:tblPr xmlns:w="urn:w"><w:tblCellSpacing w:w="5000" w:type="{width_type}"/></w:tblPr>"#
            );
            assert_eq!(parse_table_props(&xml).cell_spacing, None);
        }
    }

    #[test]
    fn parses_row_cell_spacing_override() {
        // A row can override the table default. Preserve that distinction in
        // the reader so the renderer can resolve row -> table without mutating
        // the table-wide value or special-casing the first row.
        let row = parse_row_pr(
            r#"<w:trPr xmlns:w="urn:w"><w:tblCellSpacing w:w="80" w:type="dxa"/></w:trPr>"#,
        );
        assert_eq!(row.cell_spacing, Some(4.0));
    }

    #[test]
    fn parses_cell_level_tc_margins() {
        let xml = r#"<w:tcPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                <w:tcMar>
                    <w:left w:w="200" w:type="dxa"/>
                    <w:right w:w="200" w:type="dxa"/>
                </w:tcMar>
            </w:tcPr>"#;
        let mut reader = Reader::from_str(xml);
        let mut buf = Vec::new();
        let mut cell = TableCell::default();
        let parser = DocumentParser::new();

        loop {
            buf.clear();
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) if e.name().as_ref() == b"w:tcPr" => {
                    parser
                        .parse_table_cell_properties(&mut reader, &mut cell)
                        .unwrap();
                    break;
                }
                Ok(Event::Eof) => panic!("tcPr not found"),
                Ok(_) => {}
                Err(err) => panic!("unexpected error: {err}"),
            }
        }

        let margins = cell.margins.expect("tcMar should be parsed");
        assert_eq!(margins.left, Some(10.0));
        assert_eq!(margins.right, Some(10.0));
        assert!(margins.top.is_none());
        assert!(margins.bottom.is_none());
    }

    #[test]
    fn parses_cell_vertical_alignment() {
        // `w:vAlign` (§17.4.84) is normalized to a lowercase token; absent
        // leaves it None (Word's default = top).
        let parse_valign = |xml: &str| -> Option<String> {
            let mut reader = Reader::from_str(xml);
            let mut buf = Vec::new();
            let mut cell = TableCell::default();
            let parser = DocumentParser::new();
            loop {
                buf.clear();
                match reader.read_event_into(&mut buf) {
                    Ok(Event::Start(ref e)) if e.name().as_ref() == b"w:tcPr" => {
                        parser
                            .parse_table_cell_properties(&mut reader, &mut cell)
                            .unwrap();
                        break;
                    }
                    Ok(Event::Eof) => panic!("tcPr not found"),
                    Ok(_) => {}
                    Err(err) => panic!("unexpected error: {err}"),
                }
            }
            cell.vertical_align
        };

        let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
        assert_eq!(
            parse_valign(&format!(
                r#"<w:tcPr {ns}><w:vAlign w:val="Center"/></w:tcPr>"#
            )),
            Some("center".to_string()),
            "w:vAlign must be lowercased",
        );
        assert_eq!(
            parse_valign(&format!(
                r#"<w:tcPr {ns}><w:vAlign w:val="bottom"/></w:tcPr>"#
            )),
            Some("bottom".to_string()),
        );
        assert_eq!(
            parse_valign(&format!(r#"<w:tcPr {ns}></w:tcPr>"#)),
            None,
            "absent w:vAlign must leave vertical_align unspecified",
        );
    }
}
