//! Page Setup edits preserve printer settings and attributes not modeled by
//! jade. Print titles and print areas are worksheet-scoped defined names.
use crate::xml_edit::{child_attributes, edit_child};
use crate::{Result, WriteError};
use jade_opc::OpcPackage;
use jade_xlsx_reader::{
    quote_sheet_name, CellRange, CellRef, DefinedName, PageOrientation, PageSetup,
};

const AFTER_PROPERTIES: &[&[u8]] = &[
    b"dimension",
    b"sheetViews",
    b"sheetFormatPr",
    b"cols",
    b"sheetData",
    b"extLst",
];
/// What follows `printOptions`: `pageMargins` and all that follows it.
const AFTER_PRINT_OPTIONS: &[&[u8]] = &[
    b"pageMargins",
    b"pageSetup",
    b"headerFooter",
    b"rowBreaks",
    b"colBreaks",
    b"customProperties",
    b"cellWatches",
    b"ignoredErrors",
    b"smartTags",
    b"drawing",
    b"legacyDrawing",
    b"legacyDrawingHF",
    b"picture",
    b"oleObjects",
    b"controls",
    b"webPublishItems",
    b"tableParts",
    b"extLst",
];
const AFTER_MARGINS: &[&[u8]] = &[
    b"pageSetup",
    b"headerFooter",
    b"rowBreaks",
    b"colBreaks",
    b"customProperties",
    b"cellWatches",
    b"ignoredErrors",
    b"smartTags",
    b"drawing",
    b"legacyDrawing",
    b"legacyDrawingHF",
    b"picture",
    b"oleObjects",
    b"controls",
    b"webPublishItems",
    b"tableParts",
    b"extLst",
];

pub fn set_page_setup(original: &[u8], sheet: &str, setup: &PageSetup) -> Result<Vec<u8>> {
    validate(setup)?;
    let workbook = jade_xlsx_reader::read_xlsx(original)?;
    let index = workbook
        .sheets
        .iter()
        .position(|s| s.name == sheet)
        .ok_or_else(|| WriteError::NoSuchSheet(sheet.into()))?;
    let part = &workbook.sheets[index].part_name;
    let mut package = OpcPackage::open(original)?;
    let xml = package.required_part_str(part)?;
    let xml = edit_child(&xml, b"sheetPr", AFTER_PROPERTIES, |properties| {
        child_attributes(
            properties,
            b"pageSetUpPr",
            &[("fitToPage", Some(u8::from(setup.fit_to_page).to_string()))],
            &[],
        )
    })?;
    let xml = child_attributes(
        &xml,
        b"pageSetup",
        &[
            ("paperSize", setup.paper_size.map(|v| v.to_string())),
            (
                "orientation",
                setup.orientation.map(|v| {
                    match v {
                        PageOrientation::Portrait => "portrait",
                        PageOrientation::Landscape => "landscape",
                    }
                    .to_string()
                }),
            ),
            ("scale", setup.scale.map(|v| v.to_string())),
            ("fitToWidth", setup.fit_to_width.map(|v| v.to_string())),
            ("fitToHeight", setup.fit_to_height.map(|v| v.to_string())),
            (
                "pageOrder",
                Some(
                    if setup.over_then_down {
                        "overThenDown"
                    } else {
                        "downThenOver"
                    }
                    .into(),
                ),
            ),
        ],
        &AFTER_MARGINS[1..],
    )?;
    let flags = [
        ("horizontalCentered", setup.center_horizontally),
        ("verticalCentered", setup.center_vertically),
        ("gridLines", setup.print_grid_lines),
        ("headings", setup.print_headings),
    ];
    let xml = if flags.iter().any(|(_, on)| *on) || xml.contains("printOptions") {
        let updates: Vec<_> = flags
            .iter()
            .map(|(name, on)| (*name, on.then(|| "1".to_string())))
            .collect();
        child_attributes(&xml, b"printOptions", &updates, AFTER_PRINT_OPTIONS)?
    } else {
        xml
    };
    let xml = if let Some(m) = setup.margins {
        child_attributes(
            &xml,
            b"pageMargins",
            &[
                ("left", Some(m.left.to_string())),
                ("right", Some(m.right.to_string())),
                ("top", Some(m.top.to_string())),
                ("bottom", Some(m.bottom.to_string())),
                ("header", Some(m.header.to_string())),
                ("footer", Some(m.footer.to_string())),
            ],
            AFTER_MARGINS,
        )?
    } else {
        crate::header_footer::set_child(&xml, b"pageMargins", |_| String::new(), AFTER_MARGINS)?
    };
    package.replace(part, xml.into_bytes());
    let written = package.write()?;
    let quoted = quote_sheet_name(sheet);
    let mut titles = Vec::new();
    if let Some((first, last)) = setup.title_rows {
        titles.push(format!("{quoted}!${}:${}", first + 1, last + 1));
    }
    if let Some((first, last)) = setup.title_columns {
        titles.push(format!(
            "{quoted}!${}:${}",
            column_name(first),
            column_name(last)
        ));
    }
    if titles.is_empty() {
        crate::defined_names::delete_defined_name(&written, "_xlnm.Print_Titles", Some(index))
    } else {
        crate::defined_names::set_builtin_or_user_name(
            &written,
            &DefinedName {
                name: "_xlnm.Print_Titles".into(),
                formula: titles.join(","),
                local_sheet_index: Some(index),
                hidden: false,
            },
        )
    }
}

pub fn set_print_area(original: &[u8], sheet: &str, range: Option<CellRange>) -> Result<Vec<u8>> {
    let workbook = jade_xlsx_reader::read_xlsx(original)?;
    let index = workbook
        .sheets
        .iter()
        .position(|s| s.name == sheet)
        .ok_or_else(|| WriteError::NoSuchSheet(sheet.into()))?;
    if let Some(range) = range {
        if range.start.column > range.end.column
            || range.start.row > range.end.row
            || range.end.column >= jade_xlsx_reader::model::MAX_COLUMN_COUNT
            || range.end.row >= jade_xlsx_reader::model::MAX_ROW_COUNT
        {
            return Err(WriteError::PageSetup("invalid print area".into()));
        }
        let reference = |at: CellRef| format!("${}${}", column_name(at.column), at.row + 1);
        crate::defined_names::set_builtin_or_user_name(
            original,
            &DefinedName {
                name: "_xlnm.Print_Area".into(),
                formula: format!(
                    "{}!{}:{}",
                    quote_sheet_name(sheet),
                    reference(range.start),
                    reference(range.end)
                ),
                local_sheet_index: Some(index),
                hidden: false,
            },
        )
    } else {
        crate::defined_names::delete_defined_name(original, "_xlnm.Print_Area", Some(index))
    }
}

fn validate(setup: &PageSetup) -> Result<()> {
    let invalid_range =
        |pair: Option<(u32, u32)>, limit| pair.is_some_and(|(a, b)| a > b || b >= limit);
    if setup.scale.is_some_and(|v| !(10..=400).contains(&v))
        || setup.paper_size == Some(0)
        || setup.fit_to_width.is_some_and(|v| v > 32767)
        || setup.fit_to_height.is_some_and(|v| v > 32767)
        || invalid_range(setup.title_rows, jade_xlsx_reader::model::MAX_ROW_COUNT)
        || invalid_range(
            setup.title_columns,
            jade_xlsx_reader::model::MAX_COLUMN_COUNT,
        )
        || setup.margins.is_some_and(|m| {
            [m.left, m.right, m.top, m.bottom, m.header, m.footer]
                .iter()
                .any(|v| !v.is_finite() || *v < 0.0)
        })
    {
        return Err(WriteError::PageSetup(
            "invalid paper, scaling, margins or print titles".into(),
        ));
    }
    Ok(())
}

fn column_name(mut index: u32) -> String {
    let mut letters = Vec::new();
    loop {
        letters.push(b'A' + (index % 26) as u8);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    letters.reverse();
    String::from_utf8(letters).expect("ASCII column name")
}
