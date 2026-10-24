//! Deferred range credentials and sparse permission queries for compound edits.
use crate::{
    editable_ranges::{matches, RangeGrant},
    HeadlessError, HeadlessWorkbook, Result,
};
use jade_xlsx_reader::{protected_ranges::ProtectedRange, CellRange, CellRef};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeAuthContext {
    origin: usize,
    generation: u64,
}
/// Proof of successful authentication; it contains no plaintext password and
/// changes no live grant until the whole batch is committed.
#[derive(Clone)]
pub struct RangeAuthentication {
    context: RangeAuthContext,
    grant: RangeGrant,
    definition: ProtectedRange,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RangeEditRequirement {
    pub sheet: String,
    pub ranges: Vec<CellRange>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RangeUnlockCandidate {
    pub sheet: String,
    pub part_name: String,
    pub definition: ProtectedRange,
}
impl RangeUnlockCandidate {
    pub fn key(&self) -> String {
        format!("{}\0{}", self.part_name, self.definition.name)
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum RangeUnlockStatus {
    Ready,
    Blocked,
    Prompt(Vec<RangeUnlockCandidate>),
}
fn stale() -> HeadlessError {
    HeadlessError::EditableRange("range authentication context changed".into())
}
fn intersection(a: CellRange, b: CellRange) -> Option<CellRange> {
    let start = CellRef::new(
        a.start.column.max(b.start.column),
        a.start.row.max(b.start.row),
    );
    let end = CellRef::new(a.end.column.min(b.end.column), a.end.row.min(b.end.row));
    (start.column <= end.column && start.row <= end.row).then_some(CellRange { start, end })
}
impl HeadlessWorkbook {
    pub fn range_auth_context(&self) -> RangeAuthContext {
        RangeAuthContext {
            origin: self.clipboard_origin,
            generation: self.range_auth_generation,
        }
    }
    pub fn authenticate_editable_range(
        &self,
        sheet: &str,
        name: &str,
        password: &str,
    ) -> Result<RangeAuthentication> {
        let index = self.sheet_index(sheet)?;
        let part = &self.workbook.sheets[index].part_name;
        let range = self
            .editable_ranges(sheet)?
            .iter()
            .find(|r| r.name.to_lowercase() == name.to_lowercase())
            .ok_or_else(stale)?;
        let state = &self.sheet_states[part];
        if !state.active || range.ambiguous {
            return Err(stale());
        }
        if !self.editable_range_unlocked(sheet, name)? {
            for verifier in range.password_verifiers() {
                if !verifier
                    .verifier()
                    .verifies_range(password)
                    .map_err(jade_xlsx_writer::WriteError::from)?
                {
                    return Err(jade_xlsx_writer::WriteError::IncorrectPassword.into());
                }
            }
        }
        Ok(RangeAuthentication {
            context: self.range_auth_context(),
            definition: range.clone(),
            grant: RangeGrant {
                part: part.clone(),
                name: range.name.clone(),
                passwords: range.passwords.clone(),
                descriptors: range.security_descriptors.clone(),
                stamp: state.stamp.clone(),
            },
        })
    }
    fn valid_authentication(&self, token: &RangeAuthentication) -> bool {
        token.context == self.range_auth_context()
            && self
                .sheet_states
                .get(&token.grant.part)
                .is_some_and(|state| {
                    state.definitions.as_ref().is_ok_and(|ranges| {
                        ranges.iter().any(|range| {
                            *range == token.definition
                                && matches(&token.grant, &token.grant.part, state, range)
                        })
                    })
                })
    }
    /// Validate every proof before adding any live grant. Any intervening write,
    /// history restoration or grant change invalidates pending proofs.
    pub fn apply_range_authentications(&mut self, tokens: &[RangeAuthentication]) -> Result<bool> {
        if tokens.iter().any(|token| !self.valid_authentication(token)) {
            return Err(stale());
        }
        let mut changed = false;
        for token in tokens {
            let state = &self.sheet_states[&token.grant.part];
            if !self
                .range_grants
                .iter()
                .any(|grant| matches(grant, &token.grant.part, state, &token.definition))
            {
                self.range_grants.push(token.grant.clone());
                changed = true;
            }
        }
        if changed {
            self.range_auth_generation = self.range_auth_generation.wrapping_add(1);
        }
        Ok(changed)
    }
    /// Query actual write rectangles, including multiple sheets, with staged
    /// proofs. Ready/Prompt/Blocked never changes source, history or live grants.
    pub fn range_unlock_requirements(
        &self,
        requirements: &[RangeEditRequirement],
        tokens: &[RangeAuthentication],
    ) -> Result<RangeUnlockStatus> {
        if tokens.iter().any(|token| !self.valid_authentication(token)) {
            return Err(stale());
        }
        let mut approved: BTreeMap<String, Vec<CellRange>> = BTreeMap::new();
        for token in tokens {
            approved
                .entry(token.grant.part.clone())
                .or_default()
                .extend(token.definition.ranges.iter().copied());
        }
        let mut requested = Vec::new();
        for requirement in requirements {
            let index = self.sheet_index(&requirement.sheet)?;
            for range in &requirement.ranges {
                if range.start.column > range.end.column
                    || range.start.row > range.end.row
                    || range.end.column >= jade_xlsx_reader::model::MAX_COLUMN_COUNT
                    || range.end.row >= jade_xlsx_reader::model::MAX_ROW_COUNT
                {
                    return Err(HeadlessError::BadReference(
                        "invalid edit requirement".into(),
                    ));
                }
                requested.push((index, *range));
            }
        }
        let extra = |index: usize| {
            approved
                .get(&self.workbook.sheets[index].part_name)
                .map(Vec::as_slice)
                .unwrap_or(&[])
        };
        if requested
            .iter()
            .all(|(i, r)| self.guard_cell_range_with(*i, *r, extra(*i)).is_ok())
        {
            return Ok(RangeUnlockStatus::Ready);
        }
        let mut candidates = BTreeMap::new();
        let mut possible = approved.clone();
        let mut visited = std::collections::BTreeSet::new();
        for (index, _) in &requested {
            if !visited.insert(*index) {
                continue;
            }
            let sheet = &self.workbook.sheets[*index];
            if sheet.protection.is_none() {
                continue;
            }
            let Ok(ranges) = self.editable_ranges(&sheet.name) else {
                return Ok(RangeUnlockStatus::Blocked);
            };
            for range in ranges.iter().filter(|r| {
                !r.ambiguous
                    && r.requires_password()
                    && r.password_verifiers()
                        .all(|p| p.verifier().validate().is_ok())
            }) {
                if requested
                    .iter()
                    .filter(|(i, _)| i == index)
                    .any(|(_, required)| {
                        range
                            .ranges
                            .iter()
                            .filter_map(|r| intersection(*r, *required))
                            .any(|r| {
                                self.guard_cell_range_with(*index, r, extra(*index))
                                    .is_err()
                            })
                    })
                {
                    let candidate = RangeUnlockCandidate {
                        sheet: sheet.name.clone(),
                        part_name: sheet.part_name.clone(),
                        definition: range.clone(),
                    };
                    candidates.insert(candidate.key(), candidate);
                }
                possible
                    .entry(sheet.part_name.clone())
                    .or_default()
                    .extend(range.ranges.iter().copied());
            }
        }
        if requested.iter().any(|(i, r)| {
            self.guard_cell_range_with(
                *i,
                *r,
                possible
                    .get(&self.workbook.sheets[*i].part_name)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
            )
            .is_err()
        }) {
            return Ok(RangeUnlockStatus::Blocked);
        }
        let candidates: Vec<_> = candidates.into_values().collect();
        Ok(if candidates.is_empty() {
            RangeUnlockStatus::Blocked
        } else {
            RangeUnlockStatus::Prompt(candidates)
        })
    }
}
