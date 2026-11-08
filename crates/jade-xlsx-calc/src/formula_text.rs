//! Formula text as a person types it, and as an `.xlsx` file stores it.
//!
//! Excel writes the functions newer than its 2007 file format with an
//! `_xlfn.` prefix (`_xlfn._xlws.` for the worksheet-only ones), and the
//! names `LET` and `LAMBDA` bind with `_xlpm.`: `=LET(x,2,x*3)` is stored as
//! `_xlfn.LET(_xlpm.x,2,_xlpm.x*3)`. A formula saved without them opens in
//! Excel as `#NAME?`; one shown with them is not what anybody typed.
//! [`to_file_formula`] adds what is missing and [`to_display_formula`] takes
//! it away. Both leave string literals, quoted sheet names and structured
//! references alone, and keep the text otherwise exactly as it was.

use std::collections::HashSet;

/// One lexical piece of a formula: an identifier (a function, name or
/// reference) or anything else, copied through untouched.
#[derive(Debug, PartialEq)]
enum Piece<'a> {
    Identifier(&'a str),
    Other(&'a str),
}

fn is_identifier_start(ch: char) -> bool {
    ch.is_alphabetic() || ch == '_' || ch == '\\'
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '_' | '.' | '\\' | '?')
}

/// Splits `formula` into identifiers and everything else. A string literal
/// (`"…"`, `""` escaping a quote), a quoted sheet name (`'…'`, `''`
/// escaping) and a bracketed structured reference (`[…]`, nested) are one
/// opaque piece each.
fn pieces(formula: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let chars: Vec<(usize, char)> = formula.char_indices().collect();
    let end_of = |index: usize| chars.get(index).map_or(formula.len(), |(at, _)| *at);
    let mut index = 0;
    while index < chars.len() {
        let (start, ch) = chars[index];
        let next = match ch {
            ch if ch.is_ascii_digit()
                || (ch == '.'
                    && chars
                        .get(index + 1)
                        .is_some_and(|(_, ch)| ch.is_ascii_digit())) =>
            {
                let mut cursor = index + 1;
                while chars
                    .get(cursor)
                    .is_some_and(|(_, ch)| ch.is_ascii_digit() || *ch == '.')
                {
                    cursor += 1;
                }
                if chars
                    .get(cursor)
                    .is_some_and(|(_, ch)| matches!(ch, 'e' | 'E'))
                {
                    let mut exponent = cursor + 1;
                    if chars
                        .get(exponent)
                        .is_some_and(|(_, ch)| matches!(ch, '+' | '-'))
                    {
                        exponent += 1;
                    }
                    let digits = exponent;
                    while chars
                        .get(exponent)
                        .is_some_and(|(_, ch)| ch.is_ascii_digit())
                    {
                        exponent += 1;
                    }
                    if exponent > digits {
                        cursor = exponent;
                    }
                }
                cursor
            }
            '"' | '\'' => {
                let mut cursor = index + 1;
                loop {
                    match chars.get(cursor) {
                        None => break cursor,
                        Some((_, c)) if *c == ch => {
                            if chars.get(cursor + 1).is_some_and(|(_, c)| *c == ch) {
                                cursor += 2;
                            } else {
                                break cursor + 1;
                            }
                        }
                        Some(_) => cursor += 1,
                    }
                }
            }
            '[' => {
                let mut depth = 0usize;
                let mut cursor = index;
                loop {
                    match chars.get(cursor) {
                        None => break cursor,
                        Some((_, '\''))
                            if chars.get(cursor + 1).is_some_and(|(_, ch)| {
                                matches!(ch, '[' | ']' | '#' | '@' | '\'')
                            }) =>
                        {
                            // Escaped header characters do not open or close
                            // a structured reference.
                            cursor += 2;
                            continue;
                        }
                        Some((_, '[')) => depth += 1,
                        Some((_, ']')) => {
                            depth -= 1;
                            if depth == 0 {
                                break cursor + 1;
                            }
                        }
                        Some(_) => {}
                    }
                    cursor += 1;
                }
            }
            ch if is_identifier_start(ch) => {
                let mut cursor = index + 1;
                while chars
                    .get(cursor)
                    .is_some_and(|(_, c)| is_identifier_char(*c))
                {
                    cursor += 1;
                }
                out.push(Piece::Identifier(&formula[start..end_of(cursor)]));
                index = cursor;
                continue;
            }
            _ => index + 1,
        };
        out.push(Piece::Other(&formula[start..end_of(next)]));
        index = next;
    }
    out
}

/// Whether the piece after `index` (spaces skipped) opens a call.
fn is_call(pieces: &[Piece<'_>], index: usize) -> bool {
    pieces[index + 1..]
        .iter()
        .find(|piece| !matches!(piece, Piece::Other(text) if text.trim().is_empty()))
        .is_some_and(|piece| *piece == Piece::Other("("))
}

fn bare(name: &str) -> &str {
    let mut name = name;
    for prefix in ["_xlfn._xlws.", "_xlfn.", "_xlpm."] {
        if name
            .get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        {
            name = &name[prefix.len()..];
        }
    }
    name
}

/// Identifier positions bound by LET and LAMBDA within their lexical scope.
fn bound_names(pieces: &[Piece<'_>]) -> HashSet<usize> {
    let mut names = HashSet::new();
    for (index, piece) in pieces.iter().enumerate() {
        let Piece::Identifier(name) = piece else {
            continue;
        };
        let function = bare(name).to_uppercase();
        if !(function == "LET" || function == "LAMBDA") || !is_call(pieces, index) {
            continue;
        }
        // The call's arguments, as the identifiers that make each one up.
        let mut arguments: Vec<Vec<(usize, &str)>> = vec![Vec::new()];
        let mut starts = vec![index + 1];
        let mut end = pieces.len();
        let mut depth = 0usize;
        let mut open = false;
        for (at, piece) in pieces.iter().enumerate().skip(index + 1) {
            match piece {
                Piece::Other("(") | Piece::Other("{") => {
                    depth += 1;
                    open = true;
                    if depth == 1 {
                        continue;
                    }
                }
                Piece::Other(")") | Piece::Other("}") => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end = at;
                        break;
                    }
                }
                Piece::Other(",") if depth == 1 => {
                    arguments.push(Vec::new());
                    starts.push(at + 1);
                    continue;
                }
                _ => {}
            }
            if !open {
                continue;
            }
            match piece {
                Piece::Identifier(identifier) => {
                    arguments.last_mut().unwrap().push((at, identifier))
                }
                Piece::Other(text) if text.trim().is_empty() => {}
                // Anything else in an argument: it is not a lone name.
                Piece::Other(_) => arguments.last_mut().unwrap().push((at, "")),
            }
        }
        let last = arguments.len().saturating_sub(1);
        for (position, argument) in arguments.iter().enumerate() {
            let binds = if function == "LET" {
                position % 2 == 0 && position < last
            } else {
                position < last
            };
            if let ([(declaration, identifier)], true) = (argument.as_slice(), binds) {
                if !identifier.is_empty() {
                    names.insert(*declaration);
                    // LET values are evaluated before their own name is bound.
                    // LAMBDA parameters are visible only in its body.
                    let start = if function == "LET" {
                        starts.get(position + 2).copied().unwrap_or(end)
                    } else {
                        starts[last]
                    };
                    for at in start..end {
                        if let Piece::Identifier(candidate) = pieces[at] {
                            let next = pieces[at + 1..].iter().find(|piece| {
                                !matches!(piece, Piece::Other(text) if text.trim().is_empty())
                            });
                            let previous = pieces[..at].iter().rev().find(|piece| {
                                !matches!(piece, Piece::Other(text) if text.trim().is_empty())
                            });
                            let mut tail = pieces[at + 1..].iter().filter(|piece| {
                                !matches!(piece, Piece::Other(text) if text.trim().is_empty())
                            });
                            let three_d_sheet = matches!(tail.next(), Some(Piece::Other(":")))
                                && matches!(
                                    tail.next(),
                                    Some(Piece::Identifier(_)) | Some(Piece::Other(_))
                                )
                                && matches!(tail.next(), Some(Piece::Other("!")));
                            // A sheet or table name belongs to its reference,
                            // even when a local variable has the same spelling.
                            if bare(candidate).to_uppercase() == bare(identifier).to_uppercase()
                                && !matches!(next, Some(Piece::Other("!")))
                                && !matches!(previous, Some(Piece::Other("!")))
                                && !three_d_sheet
                                && !matches!(next, Some(Piece::Other(text)) if text.starts_with('['))
                            {
                                names.insert(at);
                            }
                        }
                    }
                }
            }
        }
    }
    names
}

/// `formula` (without its `=`) as an `.xlsx` file must store it.
pub fn to_file_formula(formula: &str) -> String {
    let pieces = pieces(formula);
    let bound = bound_names(&pieces);
    let mut out = String::with_capacity(formula.len() + 16);
    for (index, piece) in pieces.iter().enumerate() {
        match piece {
            Piece::Other(text) => out.push_str(text),
            Piece::Identifier(name) => {
                let name_bare = bare(name);
                if bound.contains(&index) {
                    out.push_str("_xlpm.");
                    out.push_str(name_bare);
                } else if is_call(&pieces, index) {
                    match ironcalc_base::xlsx_function_prefix(name_bare) {
                        Some(prefix) => {
                            out.push_str(prefix);
                            out.push_str(name_bare);
                        }
                        None => out.push_str(name),
                    }
                } else {
                    out.push_str(name);
                }
            }
        }
    }
    out
}

/// `formula` as a file stores it, the way Excel shows it: without the
/// `_xlfn.`, `_xlws.` and `_xlpm.` prefixes.
pub fn to_display_formula(formula: &str) -> String {
    if !formula
        .as_bytes()
        .windows(3)
        .any(|part| part.eq_ignore_ascii_case(b"_xl"))
    {
        return formula.to_string();
    }
    pieces(formula)
        .into_iter()
        .map(|piece| match piece {
            Piece::Identifier(name) => bare(name),
            Piece::Other(text) => text,
        })
        .collect()
}

/// Whether `formula` — as typed, without its `=` — gives a function more or
/// fewer arguments than it takes, which Excel refuses as it is entered. The
/// engine evaluates the formula on its own and says so; a formula it cannot
/// read for any other reason is not this one's to refuse. An argument a
/// function never evaluates (`IF(1,2,SUM())`) goes unchecked.
pub fn wrong_argument_count(formula: &str) -> bool {
    use ironcalc_base::types::{Cell, FormulaValue};
    // Far from anything the formula could refer to, so it cannot refer to
    // itself.
    const ROW: i32 = 1_000_000;
    const COLUMN: i32 = 16_000;
    let Ok(mut model) = ironcalc_base::Model::new_empty("check", "en", "UTC", "en") else {
        return false;
    };
    let typed = format!("={}", to_file_formula(formula));
    if model
        .update_cell_with_formula(0, ROW, COLUMN, typed)
        .is_err()
    {
        return false;
    }
    model.evaluate();
    let Ok(sheet) = model.workbook.worksheet(0) else {
        return false;
    };
    let here = format!(
        "{}!{}{ROW}",
        sheet.name,
        ironcalc_base::expressions::utils::number_to_column(COLUMN).unwrap_or_default()
    );
    matches!(
        sheet.cell(ROW, COLUMN),
        Some(Cell::CellFormula { v: FormulaValue::Error { m, o, .. }, .. })
            | Some(Cell::ArrayFormula { v: FormulaValue::Error { m, o, .. }, .. })
            if m == "Wrong number of arguments" && *o == here
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponent_markers_are_numbers_rather_than_local_names() {
        for value in ["1e-3", "1E+3", ".5e-2", "2.5E3"] {
            assert_eq!(
                to_file_formula(&format!("LET(e,2,{value}+e)")),
                format!("_xlfn.LET(_xlpm.e,2,{value}+_xlpm.e)")
            );
        }
    }

    #[test]
    fn local_names_do_not_rewrite_workbook_names_or_reference_owners() {
        assert_eq!(
            to_file_formula("LET(rate,A1:A3,SUM(rate:Sheet3!A1)+SUM(rate:A3))"),
            "_xlfn.LET(_xlpm.rate,A1:A3,SUM(rate:Sheet3!A1)+SUM(_xlpm.rate:A3))"
        );
        assert_eq!(
            to_file_formula("LET(rate,2,SUM(rate:'Last Sheet'!A1)+rate)"),
            "_xlfn.LET(_xlpm.rate,2,SUM(rate:'Last Sheet'!A1)+_xlpm.rate)"
        );
        assert_eq!(
            to_file_formula("LET(rate,2,Sheet2!rate+'売上'!rate+rate)"),
            "_xlfn.LET(_xlpm.rate,2,Sheet2!rate+'売上'!rate+_xlpm.rate)"
        );
        assert_eq!(
            to_file_formula("rate+LET(rate,rate+1,rate)+rate"),
            "rate+_xlfn.LET(_xlpm.rate,rate+1,_xlpm.rate)+rate"
        );
        assert_eq!(
            to_file_formula("LET(rate,2,total,rate*3,total)+total"),
            "_xlfn.LET(_xlpm.rate,2,_xlpm.total,_xlpm.rate*3,_xlpm.total)+total"
        );
        assert_eq!(
            to_file_formula("LAMBDA(rate,rate+1)(rate)+rate"),
            "_xlfn.LAMBDA(_xlpm.rate,_xlpm.rate+1)(rate)+rate"
        );
        assert_eq!(
            to_file_formula("LET(data,2,data+data!A1+SUM(data[Value]))"),
            "_xlfn.LET(_xlpm.data,2,_xlpm.data+data!A1+SUM(data[Value]))"
        );
        assert_eq!(
            to_file_formula("LET(rate,2,LET(rate,rate+1,rate))"),
            "_xlfn.LET(_xlpm.rate,2,_xlfn.LET(_xlpm.rate,_xlpm.rate+1,_xlpm.rate))"
        );
    }

    #[test]
    fn display_prefix_matching_is_case_insensitive() {
        assert_eq!(
            to_display_formula("_XLFN.LET(_XLPM.rate,2,_XLPM.rate)"),
            "LET(rate,2,rate)"
        );
        assert_eq!(
            to_display_formula("_XLFN._XLWS.FILTER(A1:A3,A1:A3>0)"),
            "FILTER(A1:A3,A1:A3>0)"
        );
    }

    #[test]
    fn unicode_names_survive_prefix_conversion() {
        for formula in [
            "売上高合計+1",
            "SUM(売上高合計)",
            "日本語!A1",
            "_xlfn.売上高合計(A1)",
        ] {
            let displayed = to_display_formula(formula);
            assert_eq!(to_display_formula(&to_file_formula(&displayed)), displayed);
        }
        assert_eq!(to_file_formula("売上高合計+1"), "売上高合計+1");
        assert_eq!(to_display_formula("_xlpm.売上高合計"), "売上高合計");
    }

    #[test]
    fn newer_functions_gain_their_prefix_and_older_ones_do_not() {
        assert_eq!(
            to_file_formula("XLOOKUP(\"a\",A1:A3,B1:B3)"),
            "_xlfn.XLOOKUP(\"a\",A1:A3,B1:B3)"
        );
        assert_eq!(
            to_file_formula("SUM(FILTER(A1:A5,A1:A5>2))"),
            "SUM(_xlfn._xlws.FILTER(A1:A5,A1:A5>2))"
        );
        assert_eq!(to_file_formula("ifs(A1>1,1)"), "_xlfn.ifs(A1>1,1)");
        assert_eq!(to_file_formula("STDEV.S(A1:A3)"), "_xlfn.STDEV.S(A1:A3)");
        assert_eq!(to_file_formula("VLOOKUP(1,A:B,2,0)"), "VLOOKUP(1,A:B,2,0)");
        assert_eq!(to_file_formula("A1+B2*3"), "A1+B2*3");
        assert_eq!(to_file_formula("ENCODEURL(A1)"), "_xlfn.ENCODEURL(A1)");
        assert_eq!(to_file_formula("HYPERLINK(A1,B1)"), "HYPERLINK(A1,B1)");
        assert_eq!(to_file_formula("DBCS(A1)"), "DBCS(A1)");
    }

    #[test]
    fn prefixed_text_is_left_as_it_is() {
        for stored in [
            "_xlfn.XLOOKUP(1,A1:A3,B1:B3)",
            "SUM(_xlfn._xlws.SORT(A1:A3))",
            "_xlfn.LET(_xlpm.x,2,_xlpm.x*3)",
        ] {
            assert_eq!(to_file_formula(stored), stored);
        }
    }

    #[test]
    fn let_and_lambda_names_are_bound_with_xlpm() {
        assert_eq!(
            to_file_formula("LET(x,2,x*3)"),
            "_xlfn.LET(_xlpm.x,2,_xlpm.x*3)"
        );
        assert_eq!(
            to_file_formula("LET(rate, 0.1, total, A1*(1+rate), total)"),
            "_xlfn.LET(_xlpm.rate, 0.1, _xlpm.total, A1*(1+_xlpm.rate), _xlpm.total)"
        );
        assert_eq!(
            to_file_formula("LAMBDA(a,b,a+b)(1,2)"),
            "_xlfn.LAMBDA(_xlpm.a,_xlpm.b,_xlpm.a+_xlpm.b)(1,2)"
        );
    }

    #[test]
    fn strings_sheet_names_and_table_references_are_untouched() {
        assert_eq!(
            to_file_formula("CONCAT(\"IFS(\",'XLOOKUP sheet'!A1)"),
            "_xlfn.CONCAT(\"IFS(\",'XLOOKUP sheet'!A1)"
        );
        assert_eq!(
            to_file_formula("SUM(Table1[IFS])+\"say \"\"SORT(\"\"\""),
            "SUM(Table1[IFS])+\"say \"\"SORT(\"\"\""
        );
    }

    #[test]
    fn escaped_table_headers_do_not_hide_or_expose_function_calls() {
        for reference in [
            "Table1[[part']IFS(A1)]]",
            "Table1[[part'[name]]",
            "Table1[[part''name]]",
            "Table1[[part']_xlfn.FILTER(A1)]]",
        ] {
            let formula = format!("SUM({reference})+FILTER(A1:A3,A1:A3>0)");
            let stored = format!("SUM({reference})+_xlfn._xlws.FILTER(A1:A3,A1:A3>0)");
            assert_eq!(to_file_formula(&formula), stored);
            assert_eq!(to_display_formula(&stored), formula);
        }
    }

    #[test]
    fn display_drops_every_prefix_and_round_trips() {
        assert_eq!(
            to_display_formula("_xlfn.LET(_xlpm.x,2,_xlpm.x*3)"),
            "LET(x,2,x*3)"
        );
        assert_eq!(
            to_display_formula("SUM(_xlfn._xlws.FILTER(A1:A5,A1:A5>2))"),
            "SUM(FILTER(A1:A5,A1:A5>2))"
        );
        assert_eq!(to_display_formula("\"_xlfn.\"&A1"), "\"_xlfn.\"&A1");
        for typed in ["XLOOKUP(1,A1:A3,B1:B3)", "LET(x,2,x*3)", "SUM(A1:A3)"] {
            assert_eq!(to_display_formula(&to_file_formula(typed)), typed);
        }
    }
}
