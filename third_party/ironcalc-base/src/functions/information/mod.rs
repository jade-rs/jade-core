mod isomitted;

mod cell_info;

use crate::{
    calc_result::CalcResult,
    constants::COLUMN_WIDTH_FACTOR,
    expressions::{
        parser::{ArrayNode, Node},
        token::Error,
        types::CellReferenceIndex,
        utils::{number_to_column, quote_name},
    },
    get_all_timezones, host,
    model::{Model, ParsedDefinedName},
    types::HorizontalAlignment,
};

/// What INFO("system") names the operating system: "mac" and "pcdos" as
/// Excel does, the system's own name elsewhere.
#[cfg(not(target_arch = "wasm32"))]
fn get_system() -> String {
    match std::env::consts::OS {
        "macos" => "mac".to_string(),
        "windows" => "pcdos".to_string(),
        other => other.to_string(),
    }
}

#[cfg(target_arch = "wasm32")]
fn get_system() -> String {
    "browser".to_string()
}

impl<'a> Model<'a> {
    /// IS predicates inspect values without coercion and lift over arrays.
    /// Scalar errors are inspected too, rather than propagated by a cast.
    fn information_predicate(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        predicate: fn(&CalcResult) -> bool,
    ) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_values(&args[0], cell, |_, value| {
            CalcResult::Boolean(predicate(&value))
        })
    }

    pub(crate) fn fn_isnumber(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.information_predicate(args, cell, |value| matches!(value, CalcResult::Number(_)))
    }
    pub(crate) fn fn_istext(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.information_predicate(args, cell, |value| matches!(value, CalcResult::String(_)))
    }
    pub(crate) fn fn_isnontext(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.information_predicate(args, cell, |value| !matches!(value, CalcResult::String(_)))
    }
    pub(crate) fn fn_islogical(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.information_predicate(args, cell, |value| matches!(value, CalcResult::Boolean(_)))
    }
    pub(crate) fn fn_isblank(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.information_predicate(args, cell, |value| matches!(value, CalcResult::EmptyCell))
    }
    pub(crate) fn fn_iserror(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.information_predicate(args, cell, |value| {
            matches!(value, CalcResult::Error { .. })
        })
    }
    pub(crate) fn fn_iserr(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.information_predicate(
            args,
            cell,
            |value| matches!(value, CalcResult::Error { error, .. } if *error != Error::NA),
        )
    }
    pub(crate) fn fn_isna(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.information_predicate(args, cell, |value| {
            matches!(
                value,
                CalcResult::Error {
                    error: Error::NA,
                    ..
                }
            )
        })
    }

    // Returns true if it is a reference or evaluates to a reference
    // But DOES NOT evaluate
    pub(crate) fn fn_isref(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        match &args[0] {
            Node::ReferenceKind { .. }
            | Node::RangeKind { .. }
            | Node::OpRangeKind { .. }
            | Node::Range3DKind { .. } => CalcResult::Boolean(true),
            Node::FunctionKind { kind, args: _ } => CalcResult::Boolean(kind.returns_reference()),
            _ => CalcResult::Boolean(false),
        }
    }

    pub(crate) fn fn_isodd(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.parity_predicate(args, cell, true)
    }

    pub(crate) fn fn_iseven(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.parity_predicate(args, cell, false)
    }

    fn parity_predicate(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
        odd: bool,
    ) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_values(&args[0], cell, |model, value| {
            if matches!(value, CalcResult::Boolean(_)) {
                return CalcResult::new_error(Error::VALUE, cell, "Expecting number".into());
            }
            let number = match model.cast_to_number(value, cell) {
                Ok(number) => number.abs().trunc(),
                Err(error) => return error,
            };
            // Keep large numbers in the workbook representation: narrowing
            // to i64 would saturate large even numbers to the odd i64::MAX.
            CalcResult::Boolean(number % 2.0 == if odd { 1.0 } else { 0.0 })
        })
    }

    // ISFORMULA arg needs to be a reference or something that evaluates to a reference
    pub(crate) fn fn_isformula(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        if let CalcResult::Range { left, right } = self.evaluate_node_with_reference(&args[0], cell)
        {
            if left.sheet != right.sheet {
                return CalcResult::Error {
                    error: Error::ERROR,
                    origin: cell,
                    message: "3D ranges not supported".to_string(),
                };
            }
            if left.row != right.row && left.column != right.column {
                // FIXME: Implicit intersection or dynamic arrays
                return CalcResult::Error {
                    error: Error::VALUE,
                    origin: cell,
                    message: "argument must be a reference to a single cell".to_string(),
                };
            }
            let (formula_row, formula_column) =
                self.formula_information_reference(left.sheet, left.row, left.column);
            let is_formula =
                self.source_formulas
                    .contains_key(&(left.sheet, formula_row, formula_column))
                    || self
                        .get_cell_formula(left.sheet, formula_row, formula_column)
                        .ok()
                        .flatten()
                        .is_some();
            CalcResult::Boolean(is_formula)
        } else {
            CalcResult::Error {
                error: Error::VALUE,
                origin: cell,
                message: "Argument must be a reference".to_string(),
            }
        }
    }

    pub(crate) fn fn_errortype(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_values(&args[0], cell, |_, value| match value {
            CalcResult::Error { error, .. } => {
                match error {
                    Error::NULL => CalcResult::Number(1.0),
                    Error::DIV => CalcResult::Number(2.0),
                    Error::VALUE => CalcResult::Number(3.0),
                    Error::REF => CalcResult::Number(4.0),
                    Error::NAME => CalcResult::Number(5.0),
                    Error::NUM => CalcResult::Number(6.0),
                    Error::NA => CalcResult::Number(7.0),
                    Error::SPILL => CalcResult::Number(9.0),
                    Error::CALC => CalcResult::Number(14.0),
                    // IronCalc specific
                    Error::ERROR => CalcResult::Number(101.0),
                    Error::NIMPL => CalcResult::Number(102.0),
                    Error::CIRC => CalcResult::Number(104.0),
                    // Missing from Excel
                    // #GETTING_DATA => 8
                    // #CONNECT => 10
                    // #BLOCKED => 11
                    // #UNKNOWN => 12
                    // #FIELD => 13
                    // #EXTERNAL => 19
                }
            }
            _ => CalcResult::Error {
                error: Error::NA,
                origin: cell,
                message: "Not an error".to_string(),
            },
        })
    }

    // Excel believes for some reason that TYPE(A1:A7) is an array formula
    // Although we evaluate the same as Excel we cannot, ATM import this from excel
    pub(crate) fn fn_type(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        match self.evaluate_node_in_context(&args[0], cell) {
            CalcResult::String(_) => CalcResult::Number(2.0),
            CalcResult::Number(_) => CalcResult::Number(1.0),
            CalcResult::Boolean(_) => CalcResult::Number(4.0),
            CalcResult::Error { .. } => CalcResult::Number(16.0),
            CalcResult::Range { .. } | CalcResult::Array(_) => CalcResult::Number(64.0),
            CalcResult::EmptyCell => CalcResult::Number(1.0),
            CalcResult::EmptyArg => {
                // This cannot happen
                CalcResult::Number(1.0)
            }
            CalcResult::Lambda(_) => CalcResult::Error {
                error: Error::NIMPL,
                origin: cell,
                message: "Lambda type not supported yet".to_string(),
            },
        }
    }
    pub(crate) fn fn_sheet(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let arg_count = args.len();
        if arg_count > 1 {
            return CalcResult::new_args_number_error(cell);
        }
        if arg_count == 0 {
            // Sheets are 0-indexed`
            return CalcResult::Number(cell.sheet as f64 + 1.0);
        }
        // The arg could be a defined name or a table
        // let  = &args[0];
        match &args[0] {
            Node::DefinedNameKind((name, scope, _)) => {
                // Let's see if it is a defined name
                if let Some(defined_name) = self
                    .parsed_defined_names
                    .get(&(*scope, name.to_lowercase()))
                {
                    match defined_name {
                        ParsedDefinedName::CellReference(reference) => {
                            return CalcResult::Number(reference.sheet as f64 + 1.0)
                        }
                        ParsedDefinedName::RangeReference(range) => {
                            return CalcResult::Number(range.left.sheet as f64 + 1.0)
                        }
                        ParsedDefinedName::InvalidDefinedNameFormula
                        | ParsedDefinedName::FormulaDefinition(..)
                        | ParsedDefinedName::LambdaDefinition(..) => {
                            return CalcResult::Error {
                                error: Error::ERROR,
                                origin: cell,
                                message: "Invalid name".to_string(),
                            };
                        }
                    }
                } else {
                    // This should never happen
                    return CalcResult::Error {
                        error: Error::ERROR,
                        origin: cell,
                        message: "Invalid name".to_string(),
                    };
                }
            }
            Node::TableNameKind(name) => {
                // Now let's see if it is a table
                for (table_name, table) in &self.workbook.tables {
                    if table_name == name {
                        if let Some(sheet_index) = self.get_sheet_index_by_name(&table.sheet_name) {
                            return CalcResult::Number(sheet_index as f64 + 1.0);
                        } else {
                            break;
                        }
                    }
                }
            }
            Node::NamedVariableKind { name, id: _ } => {
                return CalcResult::Error {
                    error: Error::NAME,
                    origin: cell,
                    message: format!("Name not found: {name}"),
                }
            }
            arg @ (Node::ReferenceKind { .. }
            | Node::RangeKind { .. }
            | Node::Range3DKind { .. }
            | Node::FunctionKind { .. }) => {
                // A reference: the sheet it is on. A function giving one
                // (INDIRECT, OFFSET) counts; one giving a value names a sheet.
                if let CalcResult::Range { left, .. } = self.evaluate_node_with_reference(arg, cell)
                {
                    return CalcResult::Number(left.sheet as f64 + 1.0);
                }
                let sheet_name = match self.get_string(arg, cell) {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                if let Some(sheet_index) = self.get_sheet_index_by_name(&sheet_name) {
                    return CalcResult::Number(sheet_index as f64 + 1.0);
                }
            }
            arg => {
                // Now it should be the name of a sheet
                let sheet_name = match self.get_string(arg, cell) {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                if let Some(sheet_index) = self.get_sheet_index_by_name(&sheet_name) {
                    return CalcResult::Number(sheet_index as f64 + 1.0);
                }
            }
        }
        CalcResult::Error {
            error: Error::NA,
            origin: cell,
            message: "Invalid name".to_string(),
        }
    }

    pub(crate) fn fn_n(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let arg_count = args.len();
        if arg_count != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        self.map_scalar_values(&args[0], cell, |_, value| match value {
            CalcResult::Number(_) | CalcResult::Error { .. } => value,
            CalcResult::Boolean(value) => CalcResult::Number(if value { 1.0 } else { 0.0 }),
            CalcResult::Lambda(_) => CalcResult::new_error(
                Error::NIMPL,
                cell,
                "Lambda conversion not supported yet".into(),
            ),
            _ => CalcResult::Number(0.0),
        })
    }

    pub(crate) fn fn_sheets(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let arg_count = args.len();
        if arg_count > 1 {
            return CalcResult::new_args_number_error(cell);
        }
        if arg_count == 1 {
            // The sheets a reference spans: one, or each of a 3-D one's.
            return match self.evaluate_node_with_reference(&args[0], cell) {
                CalcResult::Range { left, right } => {
                    CalcResult::Number((right.sheet as f64 - left.sheet as f64).abs() + 1.0)
                }
                error @ CalcResult::Error { .. } => error,
                _ => CalcResult::Error {
                    error: Error::VALUE,
                    origin: cell,
                    message: "Argument must be a reference".to_string(),
                },
            };
        }
        let sheet_count = self.workbook.worksheets.len() as f64;
        CalcResult::Number(sheet_count)
    }

    /// CELL(info_type, [reference])
    /// NB: In Excel "info_type" is localized. Here it is always in English.
    pub(crate) fn fn_cell(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        let arg_count = args.len();
        if arg_count == 0 || arg_count > 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let reference = if arg_count == 2 {
            match self.evaluate_node_with_reference(&args[1], cell) {
                CalcResult::Range { left, right: _ } => {
                    // we just take the left cell of the range
                    left
                }
                _ => {
                    return CalcResult::Error {
                        error: Error::VALUE,
                        origin: cell,
                        message: "Argument must be a reference".to_string(),
                    }
                }
            }
        } else {
            CellReferenceIndex {
                sheet: cell.sheet,
                row: cell.row,
                column: cell.column,
            }
        };
        let info_type = match self.get_string(&args[0], cell) {
            Ok(s) => s.to_uppercase(),
            Err(e) => return e,
        };
        match info_type.as_str() {
            "ADDRESS" => {
                let column = match number_to_column(reference.column) {
                    Some(c) => c,
                    None => {
                        return CalcResult::Error {
                            error: Error::VALUE,
                            origin: cell,
                            message: "Invalid column".to_string(),
                        }
                    }
                };
                let address = format!("${}${}", column, reference.row);
                if reference.sheet == cell.sheet {
                    return CalcResult::String(address);
                }
                // On another sheet, Excel names the book and the sheet:
                // [Book1.xlsx]Sheet2!$A$1, quoted as a whole when it needs it.
                let sheet_name = match self.workbook.worksheet(reference.sheet) {
                    Ok(ws) => ws.name.clone(),
                    Err(_) => {
                        return CalcResult::Error {
                            error: Error::VALUE,
                            origin: cell,
                            message: "Invalid sheet".to_string(),
                        }
                    }
                };
                let qualified = format!("[{}]{}", self.book_file_name(), sheet_name);
                let qualified = if quote_name(&sheet_name) != sheet_name
                    || self.book_file_name().contains(' ')
                {
                    format!("'{}'", qualified.replace('\'', "''"))
                } else {
                    qualified
                };
                CalcResult::String(format!("{qualified}!{address}"))
            }
            "COL" => CalcResult::Number(reference.column as f64),
            "COLOR" | "FORMAT" | "PARENTHESES" | "PREFIX" | "PROTECT" => {
                let look = self.cell_look(reference);
                match info_type.as_str() {
                    "COLOR" => CalcResult::Number(
                        cell_info::negative_in_colour(&look.number_format) as i32 as f64,
                    ),
                    "FORMAT" => CalcResult::String(cell_info::format_code(&look.number_format)),
                    "PARENTHESES" => CalcResult::Number(cell_info::positive_in_parentheses(
                        &look.number_format,
                    ) as i32 as f64),
                    "PROTECT" => CalcResult::Number(look.locked as i32 as f64),
                    _ => {
                        // The label prefix of text: ' left, " right, ^
                        // centred, \\ filled; nothing for anything else.
                        if !matches!(self.evaluate_cell(reference), CalcResult::String(_)) {
                            return CalcResult::String(String::new());
                        }
                        let prefix = match look.horizontal {
                            HorizontalAlignment::Left => "'",
                            HorizontalAlignment::Right => "\"",
                            HorizontalAlignment::Center | HorizontalAlignment::CenterContinuous => {
                                "^"
                            }
                            HorizontalAlignment::Fill => "\\",
                            _ => "",
                        };
                        CalcResult::String(prefix.to_string())
                    }
                }
            }
            "WIDTH" => {
                // The width rounded to whole characters, and whether it is
                // the sheet's default, side by side as Excel gives them.
                let (width, default) = match host::column_width(reference.sheet, reference.column) {
                    Some(Some(width)) => width,
                    _ => {
                        let pixels = self
                            .workbook
                            .worksheet(reference.sheet)
                            .ok()
                            .and_then(|ws| {
                                let custom = ws.cols.iter().any(|col| {
                                    col.min <= reference.column
                                        && reference.column <= col.max
                                        && col.custom_width
                                });
                                ws.get_column_width(reference.column)
                                    .ok()
                                    .map(|width| (width, !custom))
                            });
                        match pixels {
                            Some((pixels, default)) => (
                                host::characters_of_width(pixels / COLUMN_WIDTH_FACTOR),
                                default,
                            ),
                            None => {
                                return CalcResult::Error {
                                    error: Error::VALUE,
                                    origin: cell,
                                    message: "Invalid sheet".to_string(),
                                }
                            }
                        }
                    }
                };
                CalcResult::Array(vec![vec![
                    ArrayNode::Number(width.trunc()),
                    ArrayNode::Boolean(default),
                ]])
            }
            "CONTENTS" => self.evaluate_cell(reference),
            "FILENAME" => {
                let workbook_name = &self.workbook.name;
                let worksheet_name = match self.workbook.worksheet(reference.sheet) {
                    Ok(ws) => &ws.name,
                    Err(_) => {
                        return CalcResult::Error {
                            error: Error::VALUE,
                            origin: cell,
                            message: "Invalid sheet".to_string(),
                        }
                    }
                };
                match host::document() {
                    // Excel gives the whole path of a saved book, and
                    // nothing before it is saved.
                    Some(document) => match (document.directory, document.file_name) {
                        (Some(directory), Some(file)) => {
                            CalcResult::String(format!("{directory}[{file}]{worksheet_name}"))
                        }
                        // A book with a name but no folder (in a browser).
                        (None, Some(file)) => {
                            CalcResult::String(format!("[{file}]{worksheet_name}"))
                        }
                        _ => CalcResult::String(String::new()),
                    },
                    None => {
                        CalcResult::String(format!("[{}.xlsx]{}", workbook_name, worksheet_name))
                    }
                }
            }
            "ROW" => CalcResult::Number(reference.row as f64),
            "TYPE" => {
                let cell_type = match self.evaluate_cell(reference) {
                    CalcResult::EmptyCell => "b",
                    CalcResult::String(_) => "l",
                    CalcResult::Number(_) => "v",
                    CalcResult::Boolean(_) => "v",
                    CalcResult::Error { .. } => "v",
                    CalcResult::Range { .. } => "v",
                    CalcResult::EmptyArg => "v",
                    CalcResult::Array(_) | CalcResult::Lambda(_) => "v",
                };
                CalcResult::String(cell_type.to_string())
            }
            _ => CalcResult::Error {
                error: Error::VALUE,
                origin: cell,
                message: "Invalid info_type".to_string(),
            },
        }
    }

    /// INFO(text_type)
    /// NB: In Excel "text_type" is localized. Here it is always in English.
    pub(crate) fn fn_info(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 1 {
            return CalcResult::new_args_number_error(cell);
        }
        let type_text = match self.get_string(&args[0], cell) {
            Ok(s) => s.to_uppercase(),
            Err(e) => return e,
        };
        // We take the release version from the git tag at build time. (v0.7.1-202-g847240b0 for example)
        // See build.rs
        let release = env!("GIT_VERSION");
        match type_text.as_str() {
            "DIRECTORY" => {
                let document = host::document();
                match document
                    .and_then(|document| document.directory)
                    .or_else(host::current_directory)
                {
                    Some(directory) => CalcResult::String(directory),
                    None => CalcResult::Error {
                        error: Error::NA,
                        origin: cell,
                        message: "No current folder".to_string(),
                    },
                }
            }
            "ORIGIN" => {
                // The top-left cell the window shows, after Lotus's "$A:".
                let (row, column) = host::document()
                    .and_then(|document| document.origin)
                    .unwrap_or((1, 1));
                let column = number_to_column(column).unwrap_or_else(|| "A".to_string());
                CalcResult::String(format!("$A:${column}${row}"))
            }
            "OSVERSION" => CalcResult::String(host::os_version()),

            // For now we just show the count of sheets in the current workbook
            "NUMFILE" => CalcResult::Number(self.workbook.worksheets.len() as f64),

            "RECALC" => CalcResult::String(
                if host::document().is_some_and(|document| document.manual) {
                    "Manual"
                } else {
                    "Automatic"
                }
                .to_string(),
            ),
            "RELEASE" => CalcResult::String(release.to_string()),
            "SYSTEM" => CalcResult::String(get_system()),
            "TIMEZONE" => CalcResult::String(self.get_timezone()),
            "TIMEZONES" => {
                let mut tzs = get_all_timezones();
                tzs.sort();
                let list: Vec<Vec<ArrayNode>> = tzs
                    .into_iter()
                    .map(|name| vec![ArrayNode::String(name.to_string())])
                    .collect();
                CalcResult::Array(list)
            }
            _ => CalcResult::Error {
                error: Error::VALUE,
                origin: cell,
                message: "Invalid type_text".to_string(),
            },
        }
    }

    /// The book's file name as CELL names it: the saved file's, else the
    /// model's name as an .xlsx.
    fn book_file_name(&self) -> String {
        host::document()
            .and_then(|document| document.file_name)
            .unwrap_or_else(|| format!("{}.xlsx", self.workbook.name))
    }

    /// The format CELL reads for a cell: the application's when it keeps
    /// the formats, else the engine's own style.
    fn cell_look(&self, reference: CellReferenceIndex) -> host::CellLook {
        if let Some(look) = host::cell_look(reference.sheet, reference.row, reference.column) {
            return look.unwrap_or(host::CellLook {
                number_format: "General".to_string(),
                horizontal: HorizontalAlignment::General,
                locked: true,
                formula_hidden: false,
            });
        }
        let style = self
            .get_style_for_cell(reference.sheet, reference.row, reference.column)
            .unwrap_or_default();
        host::CellLook {
            number_format: style.num_fmt,
            horizontal: style
                .alignment
                .map(|alignment| alignment.horizontal)
                .unwrap_or_default(),
            locked: true,
            formula_hidden: false,
        }
    }
}
