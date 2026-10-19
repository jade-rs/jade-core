//! Excel's answers to everyday formulas, checked one by one.
//!
//! Each case is a formula and what Excel (Microsoft 365, en-US, 1900 date
//! system) shows for it. They are typed into a fresh sheet beside a small
//! block of data, recalculated through the same path an edit takes, and read
//! back. The cases lean on what real workbooks use: aggregates and their
//! conditional forms, rounding, text and dates, lookups, the operators'
//! precedence, and how errors travel — including Excel's own quirks (the
//! 1900 leap day, `-2^2`, left-associative `^`), since a workbook computed
//! "correctly" but differently from Excel is wrong to the person reading it.
//!
//! Every mismatch is listed at once, so one run shows the whole gap.

use jade_xlsx_headless_api::{CellInput, CellKind};
use jade_xlsx_headless_editor::HeadlessWorkbook;

const SHEET: &str = "Conformance";

#[derive(Debug, Clone, Copy)]
enum Expect {
    /// A number, compared to 1e-9 relative (Excel shows 15 digits).
    N(f64),
    /// A number, compared to 1e-6 relative, for iterative results.
    Approx(f64),
    S(&'static str),
    B(bool),
    /// An error value, e.g. `#DIV/0!`.
    E(&'static str),
}

use Expect::*;

/// The data the formulas read:
///
/// ```text
///    A   B        C   D
/// 1  10  apple    1   Hello World
/// 2  20  banana   2
/// 3  30  cherry   3
/// 4  40  date     4
/// 5  50  elder    5
/// ```
fn data() -> Vec<(String, CellInput)> {
    let mut cells = Vec::new();
    for (row, (number, word)) in [
        (10.0, "apple"),
        (20.0, "banana"),
        (30.0, "cherry"),
        (40.0, "date"),
        (50.0, "elder"),
    ]
    .into_iter()
    .enumerate()
    {
        let row = row + 1;
        cells.push((format!("A{row}"), CellInput::Number(number)));
        cells.push((format!("B{row}"), CellInput::Text(word.into())));
        cells.push((format!("C{row}"), CellInput::Number(row as f64)));
    }
    cells.push(("D1".into(), CellInput::Text("Hello World".into())));
    cells
}

const CASES: &[(&str, Expect)] = &[
    // ---- arithmetic and precedence ----------------------------------------
    ("1+2*3", N(7.0)),
    ("2^3^2", N(64.0)),
    ("-2^2", N(4.0)),
    ("10%", N(0.1)),
    ("1+\"2\"", N(3.0)),
    ("\"a\"*1", E("#VALUE!")),
    ("1/0", E("#DIV/0!")),
    ("A1+\"x\"", E("#VALUE!")),
    ("0.1+0.2=0.3", B(true)),
    ("\"a\"<\"B\"", B(true)),
    ("\"abc\"=\"ABC\"", B(true)),
    ("TRUE>1", B(true)),
    ("1=\"1\"", B(false)),
    ("\"a\"&1", S("a1")),
    ("\"x\"&TRUE", S("xTRUE")),
    // ---- math -------------------------------------------------------------
    ("SUM(A1:A5)", N(150.0)),
    ("SUM(A1:A5,100)", N(250.0)),
    ("SUM(B1:B5)", N(0.0)),
    ("AVERAGE(A1:A5)", N(30.0)),
    ("ROUND(2.5,0)", N(3.0)),
    ("ROUND(-2.5,0)", N(-3.0)),
    ("ROUND(1.005,2)", N(1.01)),
    ("ROUND(1234.5678,-2)", N(1200.0)),
    ("ROUNDUP(3.2,0)", N(4.0)),
    ("ROUNDDOWN(-3.7,0)", N(-3.0)),
    ("MROUND(10,3)", N(9.0)),
    ("CEILING(2.5,1)", N(3.0)),
    ("FLOOR(2.5,1)", N(2.0)),
    ("CEILING.MATH(-2.5)", N(-2.0)),
    ("FLOOR.MATH(-2.5)", N(-3.0)),
    ("INT(-3.5)", N(-4.0)),
    ("TRUNC(-3.5)", N(-3.0)),
    ("MOD(-3,2)", N(1.0)),
    ("MOD(3,-2)", N(-1.0)),
    ("MOD(5,0)", E("#DIV/0!")),
    ("POWER(2,10)", N(1024.0)),
    ("SQRT(16)", N(4.0)),
    ("SQRT(-1)", E("#NUM!")),
    ("ABS(-5)", N(5.0)),
    ("SIGN(-2)", N(-1.0)),
    ("SUMPRODUCT(A1:A5,C1:C5)", N(550.0)),
    ("SUMPRODUCT((A1:A5>20)*A1:A5)", N(120.0)),
    ("SUMIF(A1:A5,\">25\")", N(120.0)),
    ("SUMIF(B1:B5,\"b*\",A1:A5)", N(20.0)),
    ("SUMIFS(A1:A5,C1:C5,\">=2\",C1:C5,\"<=4\")", N(90.0)),
    ("COUNTIF(B1:B5,\"*e*\")", N(4.0)),
    ("COUNTIF(A1:A5,30)", N(1.0)),
    ("COUNTIFS(A1:A5,\">10\",C1:C5,\"<5\")", N(3.0)),
    ("AVERAGEIF(A1:A5,\">=30\")", N(40.0)),
    ("AVERAGEIFS(A1:A5,C1:C5,\"<3\")", N(15.0)),
    ("PRODUCT(C1:C5)", N(120.0)),
    ("FACT(5)", N(120.0)),
    ("COMBIN(5,2)", N(10.0)),
    ("PERMUT(5,2)", N(20.0)),
    ("GCD(12,18)", N(6.0)),
    ("LCM(4,6)", N(12.0)),
    ("PI()", N(std::f64::consts::PI)),
    ("EXP(0)", N(1.0)),
    ("LN(1)", N(0.0)),
    ("LOG(100)", N(2.0)),
    ("LOG(8,2)", N(3.0)),
    ("LOG10(1000)", N(3.0)),
    ("QUOTIENT(7,2)", N(3.0)),
    ("SUBTOTAL(9,A1:A5)", N(150.0)),
    ("SUBTOTAL(101,A1:A5)", N(30.0)),
    ("AGGREGATE(9,6,A1:A5)", N(150.0)),
    ("AGGREGATE(9,4,A1:A5)", N(150.0)),
    ("AGGREGATE(2,0,A1:B5)", N(5.0)),
    ("AGGREGATE(3,0,A1:B5)", N(10.0)),
    ("AGGREGATE(12,6,A1:A5)", N(30.0)),
    ("AGGREGATE(14,6,A1:A5,2)", N(40.0)),
    ("AGGREGATE(14,6,A1:A5/(C1:C5<>5),1)", N(40.0)),
    ("AGGREGATE(15,6,A1:A5/(C1:C5>2),1)", N(30.0)),
    ("AGGREGATE(14,4,A1:A5/(C1:C5<>3),1)", E("#DIV/0!")),
    ("AGGREGATE(16,6,A1:A5,0.25)", N(20.0)),
    ("AGGREGATE(17,6,A1:A5,3)", N(40.0)),
    ("AGGREGATE(18,6,A1:A5,0.5)", N(30.0)),
    ("AGGREGATE(19,6,A1:A5,1)", N(15.0)),
    ("AGGREGATE(20,0,A1:A5)", E("#VALUE!")),
    ("ROMAN(1999)", S("MCMXCIX")),
    ("ARABIC(\"MCMXCIX\")", N(1999.0)),
    ("BASE(255,16)", S("FF")),
    ("DECIMAL(\"FF\",16)", N(255.0)),
    ("DEC2BIN(10)", S("1010")),
    ("HEX2DEC(\"FF\")", N(255.0)),
    ("EVEN(3)", N(4.0)),
    ("ODD(2)", N(3.0)),
    ("SUMSQ(1,2,3)", N(14.0)),
    ("RADIANS(180)", N(std::f64::consts::PI)),
    ("DEGREES(PI())", N(180.0)),
    ("ROUND(SIN(PI()/2),10)", N(1.0)),
    // ---- statistics ---------------------------------------------------------
    ("MAX(A1:A5)", N(50.0)),
    ("MIN(A1:A5)", N(10.0)),
    ("MAX(B1:B5)", N(0.0)),
    ("MEDIAN(A1:A5)", N(30.0)),
    ("MEDIAN(1,2,3,4)", N(2.5)),
    ("MODE(1,2,2,3)", N(2.0)),
    ("STDEV(A1:A5)", N(15.811388300841896)),
    ("STDEV.P(A1:A5)", N(14.142135623730951)),
    ("VAR(A1:A5)", N(250.0)),
    ("VAR.P(A1:A5)", N(200.0)),
    ("LARGE(A1:A5,2)", N(40.0)),
    ("SMALL(A1:A5,2)", N(20.0)),
    ("RANK(30,A1:A5)", N(3.0)),
    ("RANK.EQ(20,A1:A5,1)", N(2.0)),
    ("COUNT(A1:B5)", N(5.0)),
    ("COUNTA(A1:B5)", N(10.0)),
    ("COUNTBLANK(A1:A10)", N(5.0)),
    ("PERCENTILE(A1:A5,0.25)", N(20.0)),
    ("PERCENTILE.INC(A1:A5,0.1)", N(14.0)),
    ("QUARTILE(A1:A5,3)", N(40.0)),
    ("MAXIFS(A1:A5,C1:C5,\"<3\")", N(20.0)),
    ("MINIFS(A1:A5,C1:C5,\">3\")", N(40.0)),
    ("CORREL(A1:A5,C1:C5)", N(1.0)),
    ("FORECAST(6,A1:A5,C1:C5)", N(60.0)),
    ("SLOPE(A1:A5,C1:C5)", N(10.0)),
    ("INTERCEPT(A1:A5,C1:C5)", N(0.0)),
    ("NORM.S.DIST(0,TRUE)", N(0.5)),
    ("AVERAGEA(A1:B2)", N(7.5)),
    ("GEOMEAN(1,4)", N(2.0)),
    // ---- logic --------------------------------------------------------------
    ("IF(A1>5,\"big\",\"small\")", S("big")),
    ("IF(FALSE,1)", B(false)),
    ("IFS(A1>40,\"a\",A1>5,\"b\")", S("b")),
    ("IFS(A1>40,\"a\")", E("#N/A")),
    ("AND(TRUE,FALSE)", B(false)),
    ("OR(FALSE,TRUE)", B(true)),
    ("XOR(TRUE,TRUE)", B(false)),
    ("NOT(FALSE)", B(true)),
    ("IFERROR(1/0,\"err\")", S("err")),
    ("IFNA(NA(),\"na\")", S("na")),
    ("SWITCH(2,1,\"one\",2,\"two\",\"other\")", S("two")),
    ("SWITCH(9,1,\"one\",\"other\")", S("other")),
    // ---- text ---------------------------------------------------------------
    ("LEN(D1)", N(11.0)),
    ("LEFT(D1,5)", S("Hello")),
    ("LEFT(D1)", S("H")),
    ("RIGHT(D1,5)", S("World")),
    ("MID(D1,7,3)", S("Wor")),
    ("UPPER(\"abc\")", S("ABC")),
    ("LOWER(\"ABC\")", S("abc")),
    ("PROPER(\"hello wORLD\")", S("Hello World")),
    ("TRIM(\"  a   b  \")", S("a b")),
    ("CONCATENATE(\"a\",\"b\",1)", S("ab1")),
    ("CONCAT(B1:B2)", S("applebanana")),
    ("TEXTJOIN(\", \",TRUE,B1:B3)", S("apple, banana, cherry")),
    ("SUBSTITUTE(\"aaa\",\"a\",\"b\",2)", S("aba")),
    ("SUBSTITUTE(\"aaa\",\"a\",\"b\")", S("bbb")),
    ("REPLACE(\"abcdef\",2,3,\"X\")", S("aXef")),
    ("FIND(\"o\",D1)", N(5.0)),
    ("FIND(\"o\",D1,6)", N(8.0)),
    ("SEARCH(\"w\",D1)", N(7.0)),
    ("FIND(\"z\",D1)", E("#VALUE!")),
    ("REPT(\"ab\",3)", S("ababab")),
    ("EXACT(\"a\",\"A\")", B(false)),
    ("TEXT(1234.567,\"#,##0.00\")", S("1,234.57")),
    ("TEXT(0.25,\"0%\")", S("25%")),
    ("TEXT(5,\"000\")", S("005")),
    ("TEXT(DATE(2024,1,15),\"yyyy-mm-dd\")", S("2024-01-15")),
    ("TEXT(DATE(2024,1,15),\"mmm d, yyyy\")", S("Jan 15, 2024")),
    ("TEXT(DATE(2024,1,15),\"dddd\")", S("Monday")),
    ("TEXT(0.5,\"h:mm AM/PM\")", S("12:00 PM")),
    ("TEXT(-1.5,\"0.0;(0.0)\")", S("(1.5)")),
    ("TEXT(1234567,\"000-0000\")", S("123-4567")),
    ("TEXT(5,\"000\")&\"番\"", S("005番")),
    ("TEXT(0.5,\"#.##\")", S(".5")),
    ("TEXT(1234567,\"#,##0,\")", S("1,235")),
    ("TEXT(1.5,\"# ?/?\")", S("1 1/2")),
    (
        "TEXT(DATE(2024,1,15),\"[$-411]ggge\"\"年\"\"m\"\"月\"\"d\"\"日\"\"\")",
        S("令和6年1月15日"),
    ),
    ("TEXT(DATE(2024,1,15),\"[$-411]ge.m.d\")", S("R6.1.15")),
    ("TEXT(DATE(2024,1,15),\"[$-411]aaa\")", S("月")),
    ("TEXT(DATE(2024,1,15),\"[$-411]aaaa\")", S("月曜日")),
    ("TEXT(DATE(2019,4,30),\"[$-411]ggg\")", S("平成")),
    ("VALUE(\"123.5\")", N(123.5)),
    ("VALUE(\"1,234\")", N(1234.0)),
    ("NUMBERVALUE(\"1.234,5\",\",\",\".\")", N(1234.5)),
    ("CHAR(65)", S("A")),
    ("CODE(\"A\")", N(65.0)),
    ("UNICODE(\"あ\")", N(12354.0)),
    ("FIXED(1234.567,1)", S("1,234.6")),
    ("DOLLAR(1234.567,2)", S("$1,234.57")),
    ("TEXTBEFORE(\"a-b-c\",\"-\")", S("a")),
    ("TEXTAFTER(\"a-b-c\",\"-\",2)", S("c")),
    ("T(1)", S("")),
    ("N(\"a\")", N(0.0)),
    ("CLEAN(CHAR(7)&\"a\")", S("a")),
    ("LEN(\"日本語\")", N(3.0)),
    // ---- Japanese Excel (ja-JP: DBCS bytes, full/half width, eras) ---------
    ("ASC(\"ＡＢＣ１２３\")", S("ABC123")),
    ("ASC(\"カタカナ\")", S("ｶﾀｶﾅ")),
    ("ASC(\"ガ\")", S("ｶﾞ")),
    ("JIS(\"ABC123\")", S("ＡＢＣ１２３")),
    ("JIS(\"ｶﾞｷﾞ\")", S("ガギ")),
    // DBCS is Japanese Excel's JIS under its other name.
    ("DBCS(\"ABC123\")", S("ＡＢＣ１２３")),
    ("DBCS(\"ｶﾞｷﾞ\")", S("ガギ")),
    // ENCODEURL keeps letters, digits and - _ . ~; everything else is its
    // UTF-8 bytes, percent-encoded (Microsoft's own example among them).
    (
        "ENCODEURL(\"http://contoso.sharepoint.com/Finance/Profit and Loss Statement.xlsx\")",
        S("http%3A%2F%2Fcontoso.sharepoint.com%2FFinance%2FProfit%20and%20Loss%20Statement.xlsx"),
    ),
    ("ENCODEURL(\"a-b_c.d~e\")", S("a-b_c.d~e")),
    ("ENCODEURL(\"東京\")", S("%E6%9D%B1%E4%BA%AC")),
    ("ENCODEURL(12)", S("12")),
    // HYPERLINK shows its friendly name, or the link when there is none.
    (
        "HYPERLINK(\"https://example.com\",\"Example\")",
        S("Example"),
    ),
    (
        "HYPERLINK(\"https://example.com\")",
        S("https://example.com"),
    ),
    ("HYPERLINK(\"https://example.com\",42)", N(42.0)),
    ("HYPERLINK(\"#Sheet1!A1\",B1)", S("apple")),
    ("JIS(\"a b\")", S("ａ　ｂ")),
    ("LENB(\"あいう\")", N(6.0)),
    ("LENB(\"abc\")", N(3.0)),
    ("LENB(\"aあ\")", N(3.0)),
    ("LEFTB(\"あいう\",4)", S("あい")),
    ("MIDB(\"あいう\",3,2)", S("い")),
    ("REPLACEB(\"あいう\",3,2,\"X\")", S("あXう")),
    ("REPLACEB(\"abあc\",3,2,\"Z\")", S("abZc")),
    ("FINDB(\"う\",\"あいう\")", N(5.0)),
    ("DATESTRING(DATE(2024,1,15))", S("令和06年01月15日")),
    ("DATESTRING(DATE(2019,4,30))", S("平成31年04月30日")),
    ("NUMBERSTRING(123,1)", S("百二十三")),
    ("NUMBERSTRING(123,2)", S("壱百弐拾参")),
    ("NUMBERSTRING(123,3)", S("一二三")),
    ("NUMBERSTRING(1050,1)", S("千五十")),
    ("NUMBERSTRING(10000,1)", S("一万")),
    // ---- dates and times (1900 system) -------------------------------------
    ("DATE(2024,1,15)", N(45306.0)),
    ("DATE(1900,2,28)", N(59.0)),
    ("DATE(1900,3,1)", N(61.0)),
    ("DATE(2024,13,1)", N(45658.0)),
    ("YEAR(45306)", N(2024.0)),
    ("MONTH(45306)", N(1.0)),
    ("DAY(45306)", N(15.0)),
    ("DAY(60)", N(29.0)),
    ("WEEKDAY(DATE(2024,1,15))", N(2.0)),
    ("WEEKDAY(DATE(2024,1,15),2)", N(1.0)),
    ("EDATE(DATE(2024,1,31),1)", N(45351.0)),
    ("EOMONTH(DATE(2024,1,15),1)", N(45351.0)),
    ("DATEDIF(DATE(2020,1,1),DATE(2024,1,15),\"y\")", N(4.0)),
    ("DATEDIF(DATE(2020,1,1),DATE(2024,1,15),\"m\")", N(48.0)),
    ("DATEDIF(DATE(2020,1,1),DATE(2024,1,15),\"d\")", N(1475.0)),
    ("DAYS(DATE(2024,3,1),DATE(2024,2,1))", N(29.0)),
    ("NETWORKDAYS(DATE(2024,1,1),DATE(2024,1,31))", N(23.0)),
    ("WORKDAY(DATE(2024,1,5),1)", N(45299.0)),
    ("TIME(12,30,0)", N(0.5208333333333334)),
    ("HOUR(0.75)", N(18.0)),
    ("MINUTE(TIME(1,45,0))", N(45.0)),
    ("SECOND(TIME(1,45,30))", N(30.0)),
    ("DATEVALUE(\"2024-01-15\")", N(45306.0)),
    ("TIMEVALUE(\"12:00\")", N(0.5)),
    ("YEARFRAC(DATE(2024,1,1),DATE(2024,7,1))", N(0.5)),
    ("ISOWEEKNUM(DATE(2024,1,1))", N(1.0)),
    ("WEEKNUM(DATE(2024,12,31))", N(53.0)),
    ("DATE(2024,1,15)+1", N(45307.0)),
    // ---- lookup and reference ----------------------------------------------
    ("VLOOKUP(30,A1:C5,3,FALSE)", N(3.0)),
    ("VLOOKUP(35,A1:A5,1,TRUE)", N(30.0)),
    ("VLOOKUP(99,A1:C5,2,FALSE)", E("#N/A")),
    ("VLOOKUP(\"CHERRY\",B1:C5,2,FALSE)", N(3.0)),
    ("HLOOKUP(\"x\",{\"x\",\"y\";1,2},2,FALSE)", N(1.0)),
    ("INDEX(B1:B5,3)", S("cherry")),
    ("INDEX(A1:C5,2,3)", N(2.0)),
    ("MATCH(\"cherry\",B1:B5,0)", N(3.0)),
    ("MATCH(35,A1:A5,1)", N(3.0)),
    ("MATCH(\"c*\",B1:B5,0)", N(3.0)),
    ("XLOOKUP(\"date\",B1:B5,A1:A5)", N(40.0)),
    ("XLOOKUP(\"zzz\",B1:B5,A1:A5,\"none\")", S("none")),
    ("XLOOKUP(35,A1:A5,B1:B5,,-1)", S("cherry")),
    ("XMATCH(40,A1:A5)", N(4.0)),
    ("CHOOSE(2,\"a\",\"b\",\"c\")", S("b")),
    ("LOOKUP(25,A1:A5,B1:B5)", S("banana")),
    ("OFFSET(A1,2,0)", N(30.0)),
    ("SUM(OFFSET(A1,0,0,3,1))", N(60.0)),
    ("INDIRECT(\"A2\")", N(20.0)),
    ("SUM(INDIRECT(\"A1:A3\"))", N(60.0)),
    ("ROWS(A1:C5)", N(5.0)),
    ("COLUMNS(A1:C5)", N(3.0)),
    ("ROW(A3)", N(3.0)),
    ("COLUMN(C1)", N(3.0)),
    ("ADDRESS(2,3)", S("$C$2")),
    ("ADDRESS(2,3,4)", S("C2")),
    // ---- information --------------------------------------------------------
    ("ISNUMBER(A1)", B(true)),
    ("ISTEXT(B1)", B(true)),
    ("ISBLANK(A10)", B(true)),
    ("ISERROR(1/0)", B(true)),
    ("ISNA(NA())", B(true)),
    ("ISEVEN(4)", B(true)),
    ("ISODD(4)", B(false)),
    ("ISLOGICAL(TRUE)", B(true)),
    ("TYPE(\"a\")", N(2.0)),
    ("ERROR.TYPE(1/0)", N(2.0)),
    ("NA()", E("#N/A")),
    // ---- financial ----------------------------------------------------------
    ("PMT(0.05/12,360,200000)", N(-1073.6432460242797)),
    ("FV(0.05,10,-100)", N(1257.789253554884)),
    ("PV(0.05,10,-100)", N(772.1734929184818)),
    ("NPV(0.1,100,100,100)", N(248.68519909842224)),
    ("IRR({-100,60,60})", Approx(0.13066238629180732)),
    ("NPER(0.05,-100,1000)", Approx(14.206699082890463)),
    ("IPMT(0.1,1,2,100)", N(-10.0)),
    // ---- dynamic arrays and names ------------------------------------------
    ("SUM(SORT(A1:A5))", N(150.0)),
    ("INDEX(SORT(A1:A5,1,-1),1)", N(50.0)),
    ("ROWS(UNIQUE({1;1;2}))", N(2.0)),
    ("ROWS({1;1;2})", N(3.0)),
    ("COLUMNS({1,2,3,4})", N(4.0)),
    ("COLUMNS(SEQUENCE(2,3))", N(3.0)),
    ("INDEX(UNIQUE({1;1;2}),2)", N(2.0)),
    ("ROWS(UNIQUE(A1:A5))", N(5.0)),
    ("COUNTA(UNIQUE({\"a\";\"A\";\"b\"}))", N(2.0)),
    ("SUM(FILTER(A1:A5,A1:A5>25))", N(120.0)),
    ("SUM(SEQUENCE(4))", N(10.0)),
    ("LET(x,2,x*3)", N(6.0)),
    ("LAMBDA(x,x+1)(2)", N(3.0)),
    ("SUM({1,2;3,4})", N(10.0)),
];

fn conformance_book() -> HeadlessWorkbook {
    let mut book = HeadlessWorkbook::open(include_bytes!(
        "../../../public/fixtures/excel-grid-compat.xlsx"
    ))
    .unwrap();
    book.add_sheet(SHEET, None).unwrap();
    book.set_cells(SHEET, &data()).unwrap();
    book
}

fn close(expected: f64, actual: f64, tolerance: f64) -> bool {
    expected == actual || (expected - actual).abs() <= tolerance * expected.abs().max(1.0)
}

/// What the cell holds, as Excel would print it in a report.
fn describe(book: &HeadlessWorkbook, reference: &str) -> (CellKind, Option<f64>, String) {
    let cell = book.cell(SHEET, reference).unwrap();
    let text = cell.text.clone().unwrap_or_default();
    (cell.kind, cell.number, text)
}

fn matches(expect: Expect, (kind, number, text): &(CellKind, Option<f64>, String)) -> bool {
    match expect {
        N(value) => *kind == CellKind::Number && number.is_some_and(|n| close(value, n, 1e-9)),
        Approx(value) => *kind == CellKind::Number && number.is_some_and(|n| close(value, n, 1e-6)),
        S(value) => {
            *kind == CellKind::Text && text == value
                || (value.is_empty() && *kind == CellKind::Empty)
        }
        B(value) => *kind == CellKind::Bool && *number == Some(if value { 1.0 } else { 0.0 }),
        E(value) => *kind == CellKind::Error && text == value,
    }
}

#[test]
fn everyday_formulas_compute_what_excel_computes() {
    let mut book = conformance_book();
    let mut failures = Vec::new();
    for (index, (formula, expect)) in CASES.iter().enumerate() {
        let reference = format!("F{}", index + 1);
        if let Err(error) = book.set(SHEET, &reference, CellInput::Formula((*formula).into())) {
            failures.push(format!("={formula}: not accepted ({error})"));
            continue;
        }
        let got = describe(&book, &reference);
        if !matches(*expect, &got) {
            failures.push(format!(
                "={formula}: expected {expect:?}, got {:?} {:?} {:?}",
                got.0, got.1, got.2
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} formulas differ from Excel:\n{}",
        failures.len(),
        CASES.len(),
        failures.join("\n")
    );
}

/// Excel reads the functions newer than its 2007 file format only with their
/// `_xlfn.` prefix (`_xlfn._xlws.` for FILTER and SORT): saved without it, a
/// typed `XLOOKUP` opens in Excel as `#NAME?`. The formula the user sees
/// stays prefix-free.
#[test]
fn typed_modern_functions_are_saved_the_way_excel_reads_them() {
    let mut book = conformance_book();
    let typed = [
        ("F1", "XLOOKUP(\"date\",B1:B5,A1:A5)", "_xlfn.XLOOKUP("),
        ("F2", "IFS(A1>5,1)", "_xlfn.IFS("),
        (
            "F3",
            "SUM(_xlfn._xlws.FILTER(A1:A5,A1:A5>25))",
            "_xlfn._xlws.FILTER(",
        ),
        ("F4", "SUM(FILTER(A1:A5,A1:A5>25))", "_xlfn._xlws.FILTER("),
        ("F5", "AGGREGATE(9,6,A1:A5)", "_xlfn.AGGREGATE("),
        ("F6", "CONCAT(B1:B2)", "_xlfn.CONCAT("),
    ];
    for (reference, formula, _) in typed {
        book.set(SHEET, reference, CellInput::Formula(formula.into()))
            .unwrap();
    }
    let saved = book.save_bytes().unwrap();
    let package = jade_opc::OpcPackage::open(&saved).unwrap();
    let sheet_xml: String = package
        .part_names()
        .iter()
        .filter(|name| name.starts_with("xl/worksheets/sheet"))
        .filter_map(|name| package.part_str(name).ok().flatten())
        .filter(|xml| xml.contains("XLOOKUP"))
        .collect();
    let mut missing = Vec::new();
    for (reference, formula, stored) in typed {
        if !sheet_xml.contains(stored) {
            missing.push(format!(
                "{reference} ={formula}: no {stored} in the saved sheet"
            ));
        }
    }
    let reopened = HeadlessWorkbook::open(&saved).unwrap();
    for (reference, _, _) in typed {
        let formula = reopened
            .cell(SHEET, reference)
            .unwrap()
            .formula
            .unwrap_or_default();
        if formula.contains("_xlfn") {
            missing.push(format!("{reference}: shows {formula}"));
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}
