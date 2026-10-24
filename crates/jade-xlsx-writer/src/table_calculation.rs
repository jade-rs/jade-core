//! Calculated-column origins and new-row fill after structural grid edits.
use crate::{CellEdit, NewValue, Result, WriteError};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{Axis, CellRange, CellRef, CellShift, CellValue, StructureChange, Workbook};
use std::collections::{BTreeMap, BTreeSet};

enum Geometry<'a> {
    Whole(&'a StructureChange, StructureChange),
    Band(&'a CellShift, CellShift),
}
impl Geometry<'_> {
    fn sheet(&self) -> &str {
        match self {
            Self::Whole(c, _) => &c.sheet,
            Self::Band(c, _) => &c.sheet,
        }
    }
    fn same_sheet(&self, own: &str) -> bool {
        jade_xlsx_reader::formula::same_sheet_name(self.sheet(), own)
    }
    fn source(&self, at: CellRef, own: &str) -> Option<CellRef> {
        if !self.same_sheet(own) {
            return Some(at);
        }
        match self {
            Self::Whole(_, inverse) => inverse.map_cell(at),
            Self::Band(_, inverse) => inverse.map_cell(at),
        }
    }
    fn destination(&self, at: CellRef, own: &str) -> Option<CellRef> {
        if !self.same_sheet(own) {
            return Some(at);
        }
        match self {
            Self::Whole(c, _) => c.map_cell(at),
            Self::Band(c, _) => c.map_cell(at),
        }
    }
    fn axis(&self) -> StructureChange {
        match self {
            Self::Whole(c, _) => (*c).clone(),
            Self::Band(c, _) => c.axis_change(),
        }
    }
    fn formula(&self, f: &str, own: &str) -> String {
        match self {
            Self::Whole(c, _) => jade_xlsx_reader::shift_formula(f, own, c),
            Self::Band(c, _) => jade_xlsx_reader::shift_cells_formula(f, own, c),
        }
    }
}
pub(crate) fn after_structure(
    package: &mut OpcPackage,
    before: &Workbook,
    c: &StructureChange,
) -> Result<()> {
    let inverse = StructureChange {
        inserted: !c.inserted,
        ..c.clone()
    };
    follow(package, before, &Geometry::Whole(c, inverse))
}
pub(crate) fn after_cells(
    package: &mut OpcPackage,
    before: &Workbook,
    c: &CellShift,
) -> Result<()> {
    let inverse = CellShift {
        inserted: !c.inserted,
        ..c.clone()
    };
    follow(package, before, &Geometry::Band(c, inverse))
}

fn follow(package: &mut OpcPackage, before: &Workbook, geometry: &Geometry<'_>) -> Result<()> {
    if !before
        .sheets
        .iter()
        .flat_map(|s| &s.tables)
        .flat_map(|t| &t.column_properties)
        .any(|c| c.calculated_formula.is_some())
    {
        return Ok(());
    }
    // The current model supplies retained IDs and final table/data boundaries;
    // the original model supplies the template before structural rewriting.
    let current = jade_xlsx_reader::read_xlsx(&package.write()?)?;
    let axis = geometry.axis();
    let mut filled = 0usize;
    for sheet in &current.sheets {
        let cells = sheet
            .rows
            .iter()
            .flat_map(|r| &r.cells)
            .map(|c| (c.reference, c))
            .collect::<BTreeMap<_, _>>();
        let mut edits = Vec::new();
        for table in &sheet.tables {
            let Some((old_sheet, old)) = before.sheets.iter().find_map(|s| {
                s.tables
                    .iter()
                    .find(|t| t.part_name == table.part_name)
                    .map(|t| (s, t))
            }) else {
                continue;
            };
            let first = table.reference.start.row + table.header_row_count;
            let last = table
                .reference
                .end
                .row
                .saturating_sub(table.totals_row_count);
            if first > last {
                continue;
            }
            let old_first = old.reference.start.row + old.header_row_count;
            let old_data = CellRange {
                start: CellRef::new(old.reference.start.column, old_first),
                end: CellRef::new(
                    old.reference.end.column,
                    old.reference.end.row.saturating_sub(old.totals_row_count),
                ),
            };
            let mut formulas = BTreeMap::new();
            for (column, properties) in table.column_properties.iter().enumerate() {
                let Some((old_column, old_properties)) = old
                    .column_properties
                    .iter()
                    .enumerate()
                    .find(|(_, p)| p.id == properties.id)
                else {
                    continue;
                };
                let Some(template) = &old_properties.calculated_formula else {
                    continue;
                };
                let original =
                    CellRef::new(old.reference.start.column + old_column as u32, old_first);
                let target = CellRef::new(table.reference.start.column + column as u32, first);
                let mut source = geometry.source(target, &sheet.name).unwrap_or_else(|| {
                    // Inserting at the first data row places the new origin
                    // inside a gap. Use the original row that moved below it.
                    let mut at = original;
                    if axis.inserted && axis.axis == Axis::Rows && geometry.same_sheet(&sheet.name)
                    {
                        at.row = axis.at;
                    }
                    at
                });
                source.column = original.column;
                let at_source = jade_xlsx_reader::translate_formula(template, original, source);
                let moved = geometry.formula(&at_source, &old_sheet.name);
                let destination = geometry.destination(source, &sheet.name).unwrap_or(target);
                let rebased = jade_xlsx_reader::translate_formula(&moved, destination, target);
                if properties.calculated_formula.as_deref() != Some(rebased.as_str()) {
                    formulas.insert(properties.id, rebased.clone());
                }

                // Consider only inserted rows and synthetic boundary rows. A
                // pre-existing blank or literal in a calculated column is an
                // exception and must remain one.
                let mut candidates = BTreeSet::from([first, last]);
                if geometry.same_sheet(&sheet.name) && axis.axis == Axis::Rows && axis.inserted {
                    let lo = first.max(axis.at);
                    let hi = last.min(axis.at.saturating_add(axis.count - 1));
                    if lo <= hi
                        && geometry
                            .source(CellRef::new(target.column, lo), &sheet.name)
                            .is_none()
                    {
                        if u64::from(hi - lo) + 1 > crate::clipboard::CLIPBOARD_CELL_LIMIT {
                            return Err(WriteError::CellShift(
                                "calculated-column fill exceeds the cell limit".into(),
                            ));
                        }
                        candidates.extend(lo..=hi);
                    }
                }
                for row in candidates {
                    let at = CellRef::new(target.column, row);
                    if geometry
                        .source(at, &sheet.name)
                        .is_some_and(|source| old_data.contains(source))
                    {
                        continue;
                    }
                    if cells
                        .get(&at)
                        .is_some_and(|c| c.formula.is_some() || c.value != CellValue::Empty)
                    {
                        continue;
                    }
                    filled += 1;
                    if filled as u64 > crate::clipboard::CLIPBOARD_CELL_LIMIT {
                        return Err(WriteError::CellShift(
                            "calculated-column fill exceeds the cell limit".into(),
                        ));
                    }
                    edits.push(CellEdit {
                        reference: at,
                        value: NewValue::Formula(jade_xlsx_reader::translate_formula(
                            &rebased, target, at,
                        )),
                        style_index: None,
                    });
                }
            }
            if !formulas.is_empty() {
                let xml = package.required_part_str(&table.part_name)?;
                package.replace(
                    &table.part_name,
                    crate::tables::rebase_calculated_formulas(&xml, &formulas)?.into_bytes(),
                );
            }
        }
        if !edits.is_empty() {
            let xml = package.required_part_str(&sheet.part_name)?;
            package.replace(
                &sheet.part_name,
                crate::patch_sheet(&xml, &edits)?.into_bytes(),
            );
        }
    }
    Ok(())
}
