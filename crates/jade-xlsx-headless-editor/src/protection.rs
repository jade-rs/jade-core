use crate::{HeadlessError, HeadlessWorkbook, Result};
impl HeadlessWorkbook {
    /// Whole rectangles, including complete axes, checked from sparse default
    /// style bands and authorized range boundaries rather than visiting blanks.
    pub(crate) fn guard_cell_range(
        &self,
        index: usize,
        range: jade_xlsx_reader::CellRange,
    ) -> Result<()> {
        self.guard_cell_range_with(index, range, &[])
    }
    pub(crate) fn guard_cell_range_with(
        &self,
        index: usize,
        range: jade_xlsx_reader::CellRange,
        extra: &[jade_xlsx_reader::CellRange],
    ) -> Result<()> {
        use jade_xlsx_reader::CellRef;
        let sheet = &self.workbook.sheets[index];
        if sheet.protection.is_none() {
            return Ok(());
        }
        let denied = || HeadlessError::Protected {
            sheet: sheet.name.clone(),
            what: "the cell is locked",
        };
        let unlocked = |style: u32| {
            self.workbook
                .styles
                .cell_formats
                .get(style as usize)
                .is_some_and(|f| f.unlocked)
        };
        let allowed: Vec<_> = self
            .editable_cell_ranges(index)
            .into_iter()
            .chain(extra.iter().copied())
            .filter(|r| {
                r.start.row <= range.end.row
                    && range.start.row <= r.end.row
                    && r.start.column <= range.end.column
                    && range.start.column <= r.end.column
            })
            .collect();
        let mut rows = vec![range.start.row, range.end.row + 1];
        let mut columns = vec![range.start.column, range.end.column + 1];
        let first_relevant = sheet
            .rows
            .partition_point(|row| row.index < range.start.row);
        let last_relevant = sheet.rows.partition_point(|row| row.index <= range.end.row);
        for row in &sheet.rows[first_relevant..last_relevant] {
            if row.custom_format
                && row.style_index.is_some()
                && range.start.row <= row.index
                && row.index <= range.end.row
            {
                rows.extend([row.index, row.index + 1]);
            }
        }
        for col in &sheet.columns {
            if col.style_index.is_some() {
                columns.extend(
                    [col.first, col.last.saturating_add(1)]
                        .into_iter()
                        .filter(|c| range.start.column < *c && *c <= range.end.column),
                );
            }
        }
        for r in &allowed {
            rows.extend(
                [r.start.row, r.end.row + 1]
                    .into_iter()
                    .filter(|v| range.start.row < *v && *v <= range.end.row),
            );
            columns.extend(
                [r.start.column, r.end.column + 1]
                    .into_iter()
                    .filter(|v| range.start.column < *v && *v <= range.end.column),
            );
        }
        rows.sort_unstable();
        rows.dedup();
        columns.sort_unstable();
        columns.dedup();
        for r in rows.windows(2) {
            let first = sheet.rows.partition_point(|row| row.index < r[0]);
            let last = sheet.rows.partition_point(|row| row.index < r[1]);
            let records = &sheet.rows[first..last];
            let row = records.first().filter(|row| row.index == r[0]);
            for c in columns.windows(2) {
                let at = CellRef::new(c[0], r[0]);
                if allowed.iter().any(|range| range.contains(at)) {
                    continue;
                }
                let base = sheet
                    .effective_style_index_in_row(at, None, row)
                    .unwrap_or(0);
                let mut overrides = std::collections::HashSet::new();
                for row in records {
                    for cell in &row.cells {
                        if c[0] <= cell.reference.column && cell.reference.column < c[1] {
                            if let Some(style) = cell.style_index {
                                if !unlocked(style) {
                                    return Err(denied());
                                }
                                overrides.insert(cell.reference);
                            }
                        }
                    }
                }
                let area = u64::from(r[1] - r[0]) * u64::from(c[1] - c[0]);
                if !unlocked(base) && (overrides.len() as u64) < area {
                    return Err(denied());
                }
            }
        }
        Ok(())
    }
    fn objects_locked(&self, index: usize) -> Result<bool> {
        Ok(self
            .worksheet_protection(&self.workbook.sheets[index].name)?
            .is_some_and(|p| !p.allowed.objects))
    }
    pub(crate) fn guarded_drawing(
        &self,
        index: usize,
        id: u32,
        text: bool,
        what: &'static str,
    ) -> Result<(
        String,
        String,
        jade_xlsx_reader::object_protection::DrawingSource,
    )> {
        let sheet = &self.workbook.sheets[index];
        let missing = || HeadlessError::NoSuchShape(id.to_string());
        let models: Vec<_> = sheet.shapes.iter().filter(|s| s.id == id).collect();
        if models.len() != 1 {
            return Err(missing());
        }
        let (part, xml) = self.drawing_part(index)?.ok_or_else(missing)?;
        let source =
            jade_xlsx_reader::object_protection::drawing_source(&xml, id)?.ok_or_else(missing)?;
        if source.name != models[0].name {
            return Err(missing());
        }
        if self.objects_locked(index)?
            && if text {
                source.locks.text_locked
            } else {
                source.locks.locked
            }
        {
            return Err(HeadlessError::Protected {
                sheet: sheet.name.clone(),
                what,
            });
        }
        Ok((part, xml, source))
    }
    pub(crate) fn guard_note(
        &self,
        index: usize,
        at: jade_xlsx_reader::CellRef,
        text: bool,
    ) -> Result<()> {
        if !self.objects_locked(index)? {
            return Ok(());
        }
        let sheet = &self.workbook.sheets[index];
        let denied = || HeadlessError::Protected {
            sheet: sheet.name.clone(),
            what: "editing notes is not allowed",
        };
        if sheet.comment_at(at).is_none() {
            return Err(denied());
        }
        let p = jade_opc::OpcPackage::open(&self.source)?;
        let rels = p.required_part_str(&jade_opc::rels_part_name(&sheet.part_name))?;
        let rels = jade_xlsx_reader::object_protection::owned_relationships(&rels)?;
        let comments: Vec<_> = rels
            .by_type_suffix("/comments")
            .into_iter()
            .filter(|r| Self::owned_relationship(r, "comments"))
            .collect();
        if comments.len() != 1 {
            return Err(denied());
        }
        let sheet_xml = p.required_part_str(&sheet.part_name)?;
        let id =
            jade_xlsx_reader::object_protection::worksheet_drawing_relationship(&sheet_xml, true)?
                .ok_or_else(denied)?;
        let vml = rels
            .relationship(&id)
            .filter(|r| Self::owned_relationship(r, "vmlDrawing"))
            .ok_or_else(denied)?;
        let xml = p.required_part_str(&jade_opc::resolve_target(
            &sheet.part_name,
            &comments[0].target,
        ))?;
        if !jade_xlsx_reader::object_protection::comments_source(&xml)?
            .comments
            .iter()
            .any(|c| c.at == Some(at))
        {
            return Err(denied());
        }
        let xml = p.required_part_str(&jade_opc::resolve_target(&sheet.part_name, &vml.target))?;
        let notes = jade_xlsx_reader::object_protection::note_sources(&xml)?;
        let Some(note) = notes.iter().find(|n| n.at == at) else {
            return Err(denied());
        };
        if note.locks.ambiguous
            || if text {
                note.locks.text_locked
            } else {
                note.locks.locked
            }
        {
            return Err(denied());
        }
        Ok(())
    }
    pub(crate) fn owned_relationship(r: &jade_opc::Relationship, kind: &str) -> bool {
        jade_xlsx_reader::object_protection::office_relationship(r, kind)
    }
    /// Imported operation permissions, including independent object/scenario
    /// locks. This read-only query follows the reader's selected MCE branch.
    pub fn worksheet_protection(
        &self,
        sheet: &str,
    ) -> Result<Option<jade_xlsx_reader::sheet_protection::ProtectionState>> {
        let index = self.sheet_index(sheet)?;
        let part = &self.workbook.sheets[index].part_name;
        self.sheet_states[part]
            .protection
            .clone()
            .map_err(|e| jade_xlsx_writer::WriteError::SheetProtection(e).into())
    }

    pub(crate) fn guard_objects(&self, index: usize, what: &'static str) -> Result<()> {
        let sheet = &self.workbook.sheets[index];
        if self
            .worksheet_protection(&sheet.name)?
            .is_some_and(|p| !p.allowed.objects)
        {
            return Err(HeadlessError::Protected {
                sheet: sheet.name.clone(),
                what,
            });
        }
        Ok(())
    }
    pub fn workbook_protection(
        &self,
    ) -> Result<Option<jade_xlsx_writer::protection::WorkbookProtectionInfo>> {
        Ok(jade_xlsx_writer::protection::workbook_protection(
            &self.source,
        )?)
    }
    pub fn protect_workbook(&mut self, password: &str, salt: &[u8; 16]) -> Result<bool> {
        let written = jade_xlsx_writer::protection::protect_workbook(&self.source, password, salt)?;
        self.commit_protection(written)
    }
    pub fn unprotect_workbook(&mut self, password: &str) -> Result<bool> {
        let written = jade_xlsx_writer::protection::unprotect_workbook(&self.source, password)?;
        self.commit_protection(written)
    }
    pub fn sheet_protection(
        &self,
        sheet: &str,
    ) -> Result<Option<jade_xlsx_writer::sheet_protection::SheetProtectionInfo>> {
        let index = self.sheet_index(sheet)?;
        Ok(jade_xlsx_writer::sheet_protection::sheet_protection(
            &self.source,
            &self.workbook.sheets[index].part_name,
        )?)
    }
    pub fn protect_sheet(
        &mut self,
        sheet: &str,
        allowed: jade_xlsx_reader::SheetProtection,
        password: &str,
        salt: &[u8; 16],
    ) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        let written = jade_xlsx_writer::sheet_protection::protect_sheet(
            &self.source,
            &self.workbook.sheets[index].part_name,
            allowed,
            password,
            salt,
        )?;
        self.commit_protection(written)
    }
    pub fn unprotect_sheet(&mut self, sheet: &str, password: &str) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        let written = jade_xlsx_writer::sheet_protection::unprotect_sheet(
            &self.source,
            &self.workbook.sheets[index].part_name,
            password,
        )?;
        self.commit_protection(written)
    }
    fn commit_protection(&mut self, written: Vec<u8>) -> Result<bool> {
        if written == self.source {
            return Ok(false);
        }
        self.replace_parts_from_bytes(written)?;
        Ok(true)
    }
}
