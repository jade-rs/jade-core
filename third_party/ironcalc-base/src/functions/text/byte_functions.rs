// FINDB, LEFTB, LENB, MIDB, REPLACEB, RIGHTB, SEARCHB
//
// Jade: these count bytes as a Japanese (DBCS) Excel does — Shift_JIS, one
// byte for ASCII and half-width katakana, two for the rest
// (`functions/japanese.rs`). REPLACEB still delegates to REPLACE.

use crate::{
    calc_result::CalcResult,
    expressions::{parser::Node, types::CellReferenceIndex},
    model::Model,
};

impl<'a> Model<'a> {
    pub(crate) fn fn_findb(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let found = self.fn_find(args, cell);
        self.dbcs_position(found, args, cell)
    }

    pub(crate) fn fn_leftb(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.dbcs_slice(args, cell, 'L')
    }

    pub(crate) fn fn_lenb(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.dbcs_lenb(args, cell)
    }

    pub(crate) fn fn_midb(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.dbcs_slice(args, cell, 'M')
    }

    pub(crate) fn fn_replaceb(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.fn_replace(args, cell)
    }

    pub(crate) fn fn_rightb(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.dbcs_slice(args, cell, 'R')
    }

    pub(crate) fn fn_searchb(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let found = self.fn_search(args, cell);
        self.dbcs_position(found, args, cell)
    }
}
