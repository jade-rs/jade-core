//! Cached worksheet permissions, editable ranges and unsaved authentication grants.
use crate::{HeadlessError, HeadlessWorkbook, Result};
use jade_xlsx_reader::{
    protected_ranges::{ProtectedRange, RangePassword},
    CellRange, CellRef,
};
use std::collections::BTreeMap;
#[derive(Clone)]
pub(crate) struct SheetState {
    pub(crate) protection:
        std::result::Result<Option<jade_xlsx_reader::sheet_protection::ProtectionState>, String>,
    pub(crate) definitions: std::result::Result<Vec<ProtectedRange>, String>,
    pub(crate) stamp: String,
    pub(crate) active: bool,
    /// The sheet part's zip entry when this was read: the same entry, the
    /// same state, so a write elsewhere does not read the part again.
    pub(crate) fingerprint: Option<jade_opc::EntryFingerprint>,
}
#[derive(Clone)]
pub(crate) struct RangeGrant {
    pub(crate) part: String,
    pub(crate) name: String,
    pub(crate) passwords: Vec<RangePassword>,
    pub(crate) descriptors: Vec<String>,
    pub(crate) stamp: String,
}
fn error(s: &str) -> HeadlessError {
    HeadlessError::EditableRange(s.into())
}
pub(crate) fn matches(
    grant: &RangeGrant,
    part: &str,
    state: &SheetState,
    range: &ProtectedRange,
) -> bool {
    state.active
        && !range.ambiguous
        && grant.part == part
        && grant.name == range.name
        && grant.passwords == range.passwords
        && grant.descriptors == range.security_descriptors
        && grant.stamp == state.stamp
}
impl HeadlessWorkbook {
    /// After a write that only typed values and their recalculation into
    /// the sheets — no protection or protected range can have changed — each
    /// sheet's state, read from the package `before` was taken of, stays as
    /// it was for the same part in the package `after` was taken of; the
    /// refresh then reads nothing.
    pub(crate) fn carry_sheet_states(
        &mut self,
        before: &[jade_opc::EntryFingerprint],
        after: &[jade_opc::EntryFingerprint],
    ) {
        for (part, state) in self.sheet_states.iter_mut() {
            let current = state
                .fingerprint
                .as_ref()
                .is_some_and(|fingerprint| before.contains(fingerprint));
            if !current {
                continue;
            }
            if let Some(entry) = after.iter().find(|entry| entry.0 == *part) {
                state.fingerprint = Some(entry.clone());
            }
        }
    }

    pub(crate) fn refresh_sheet_states(&mut self) {
        // Only the sheet parts that changed are read again, each on its own:
        // inflating the whole package on every write was most of a write's
        // cost on a large sheet.
        let names: Vec<String> = self
            .workbook
            .sheets
            .iter()
            .map(|sheet| sheet.part_name.clone())
            .collect();
        let fingerprints: BTreeMap<String, jade_opc::EntryFingerprint> =
            jade_opc::entry_fingerprints(&self.source, &names)
                .map(|found| {
                    found
                        .into_iter()
                        .map(|entry| (entry.0.clone(), entry))
                        .collect()
                })
                .unwrap_or_default();
        let mut states = BTreeMap::new();
        for sheet in &self.workbook.sheets {
            let fingerprint = fingerprints.get(&sheet.part_name).cloned();
            if let Some(previous) = self
                .sheet_states
                .get(&sheet.part_name)
                .filter(|state| fingerprint.is_some() && state.fingerprint == fingerprint)
            {
                let mut state = previous.clone();
                state.active = sheet.protection.is_some();
                states.insert(sheet.part_name.clone(), state);
                continue;
            }
            let xml = jade_opc::read_part_str(&self.source, &sheet.part_name)
                .and_then(|xml| {
                    xml.ok_or_else(|| jade_opc::OpcError::MissingPart(sheet.part_name.clone()))
                })
                .map_err(|e| e.to_string());
            let protection = xml.as_ref().map_err(Clone::clone).and_then(|xml| {
                if !xml.contains("sheetProtection") {
                    return Ok(None);
                }
                jade_xlsx_reader::sheet_protection::read(xml).map_err(|e| e.to_string())
            });
            let parsed = (|| -> std::result::Result<_, String> {
                let xml = xml.as_ref().map_err(Clone::clone)?;
                if !xml.contains("protectedRange") {
                    return Ok((Vec::new(), String::new()));
                }
                let definitions = jade_xlsx_reader::protected_ranges::protected_ranges(xml)
                    .map_err(|e| e.to_string())?;
                let stamp = jade_xlsx_reader::protected_ranges::protection_stamp(xml)
                    .map_err(|e| e.to_string())?;
                Ok((definitions, stamp))
            })();
            let (definitions, stamp) = match parsed {
                Ok((value, stamp)) => (Ok(value), stamp),
                Err(e) => (Err(e), String::new()),
            };
            states.insert(
                sheet.part_name.clone(),
                SheetState {
                    protection,
                    definitions,
                    stamp,
                    active: sheet.protection.is_some(),
                    fingerprint,
                },
            );
        }
        self.sheet_states = states;
        self.prune_range_grants();
    }
    pub(crate) fn prune_range_grants(&mut self) {
        let states = &self.sheet_states;
        self.range_grants.retain(|grant| {
            states.get(&grant.part).is_some_and(|state| {
                state.definitions.as_ref().is_ok_and(|ranges| {
                    ranges
                        .iter()
                        .any(|range| matches(grant, &grant.part, state, range))
                })
            })
        });
    }
    pub fn editable_ranges(&self, sheet: &str) -> Result<&[ProtectedRange]> {
        let index = self.sheet_index(sheet)?;
        let part = &self.workbook.sheets[index].part_name;
        self.sheet_states
            .get(part)
            .ok_or_else(|| error("missing worksheet range state"))?
            .definitions
            .as_deref()
            .map_err(|e| HeadlessError::EditableRange(e.clone()))
    }
    /// Valid password-bearing alternatives for a currently forbidden locked
    /// cell. This query uses cached permissions and never edits or authenticates.
    pub fn range_unlock_candidates(&self, sheet: &str, at: CellRef) -> Result<Vec<ProtectedRange>> {
        if at.column >= jade_xlsx_reader::model::MAX_COLUMN_COUNT
            || at.row >= jade_xlsx_reader::model::MAX_ROW_COUNT
        {
            return Err(HeadlessError::BadReference(format!(
                "{},{}",
                at.column, at.row
            )));
        }
        let index = self.sheet_index(sheet)?;
        if self.guard_cells(index, [at]).is_ok() {
            return Ok(Vec::new());
        }
        Ok(self
            .editable_ranges(sheet)?
            .iter()
            .filter(|range| {
                !range.ambiguous
                    && range.requires_password()
                    && range.ranges.iter().any(|r| r.contains(at))
                    && range
                        .password_verifiers()
                        .all(|password| password.verifier().validate().is_ok())
            })
            .cloned()
            .collect())
    }
    pub(crate) fn editable_cell_ranges(&self, index: usize) -> Vec<CellRange> {
        let part = &self.workbook.sheets[index].part_name;
        let Some(state) = self.sheet_states.get(part) else {
            return Vec::new();
        };
        let Ok(ranges) = &state.definitions else {
            return Vec::new();
        };
        ranges
            .iter()
            .filter(|range| {
                !range.ambiguous
                    && (!range.requires_password()
                        || self
                            .range_grants
                            .iter()
                            .any(|grant| matches(grant, part, state, range)))
            })
            .flat_map(|range| range.ranges.iter().copied())
            .collect()
    }
    pub(crate) fn cell_in_editable_range(&self, index: usize, at: CellRef) -> bool {
        let part = &self.workbook.sheets[index].part_name;
        self.sheet_states.get(part).is_some_and(|state| {
            state.definitions.as_ref().is_ok_and(|ranges| {
                ranges.iter().any(|range| {
                    !range.ambiguous
                        && range.ranges.iter().any(|r| r.contains(at))
                        && (!range.requires_password()
                            || self
                                .range_grants
                                .iter()
                                .any(|grant| matches(grant, part, state, range)))
                })
            })
        })
    }
    pub fn editable_range_unlocked(&self, sheet: &str, name: &str) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        let part = &self.workbook.sheets[index].part_name;
        let ranges = self.editable_ranges(sheet)?;
        let range = ranges
            .iter()
            .find(|r| r.name.to_lowercase() == name.to_lowercase())
            .ok_or_else(|| error("no editable range has this name"))?;
        let state = &self.sheet_states[part];
        Ok(state.active
            && !range.ambiguous
            && (!range.requires_password()
                || self
                    .range_grants
                    .iter()
                    .any(|grant| matches(grant, part, state, range))))
    }
    /// Authenticate a range for this live workbook only. Source/history/redo stay unchanged.
    pub fn unlock_editable_range(
        &mut self,
        sheet: &str,
        name: &str,
        password: &str,
    ) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        let part = self.workbook.sheets[index].part_name.clone();
        let ranges = self.editable_ranges(sheet)?;
        let range = ranges
            .iter()
            .find(|r| r.name.to_lowercase() == name.to_lowercase())
            .ok_or_else(|| error("no editable range has this name"))?;
        if range.ambiguous {
            return Err(error("ambiguous editable-range metadata"));
        }
        if !self.sheet_states[&part].active || self.editable_range_unlocked(sheet, name)? {
            return Ok(false);
        }
        let token = self.authenticate_editable_range(sheet, name, password)?;
        self.apply_range_authentications(&[token])
    }
    pub fn lock_editable_ranges(&mut self, sheet: &str) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        let part = &self.workbook.sheets[index].part_name;
        let before = self.range_grants.len();
        self.range_grants.retain(|grant| grant.part != *part);
        let changed = before != self.range_grants.len();
        if changed {
            self.range_auth_generation = self.range_auth_generation.wrapping_add(1);
        }
        Ok(changed)
    }
    pub fn set_editable_range(
        &mut self,
        sheet: &str,
        old_name: Option<&str>,
        name: &str,
        ranges: &[CellRange],
        password: Option<(&str, &[u8; 16])>,
    ) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        let bytes = jade_xlsx_writer::protected_ranges::set_editable_range(
            &self.source,
            &self.workbook.sheets[index].part_name,
            old_name,
            name,
            ranges,
            password,
        )?;
        if bytes == self.source {
            return Ok(false);
        }
        self.replace_parts_from_bytes(bytes)?;
        Ok(true)
    }
    pub fn delete_editable_range(&mut self, sheet: &str, name: &str) -> Result<bool> {
        let index = self.sheet_index(sheet)?;
        let bytes = jade_xlsx_writer::protected_ranges::delete_editable_range(
            &self.source,
            &self.workbook.sheets[index].part_name,
            name,
        )?;
        if bytes == self.source {
            return Ok(false);
        }
        self.replace_parts_from_bytes(bytes)?;
        Ok(true)
    }
}
