//! Only collections at their stylesheet owner path supply table/pivot formats.
use crate::{
    conditional_format::DifferentialStyle,
    model::{TableStyleDefinition, TableStyleElement},
    pivot::xml::{document, Node, MAIN},
    Result, XlsxError,
};
pub(super) struct Collections {
    pub differential: Vec<DifferentialStyle>,
    pub tables: Vec<TableStyleDefinition>,
    pub default_pivot: Option<String>,
}
fn bad(e: XlsxError) -> XlsxError {
    match e {
        XlsxError::BadPivot(s) => XlsxError::BadStyles(s),
        e => e,
    }
}
fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}
fn allowed(parent: &str, child: &str) -> bool {
    match parent {
        "dxf" => matches!(
            child,
            "font" | "fill" | "border" | "numFmt" | "alignment" | "protection"
        ),
        "font" => matches!(
            child,
            "b" | "i"
                | "u"
                | "strike"
                | "color"
                | "name"
                | "sz"
                | "vertAlign"
                | "family"
                | "scheme"
                | "charset"
                | "outline"
                | "shadow"
                | "condense"
                | "extend"
        ),
        "fill" => matches!(child, "patternFill" | "gradientFill"),
        "patternFill" => matches!(child, "fgColor" | "bgColor"),
        "border" => matches!(
            child,
            "left" | "right" | "top" | "bottom" | "vertical" | "horizontal" | "diagonal"
        ),
        "left" | "right" | "top" | "bottom" | "vertical" | "horizontal" | "diagonal" => {
            child == "color"
        }
        _ => false,
    }
}
fn serialize(n: &Node, out: &mut String) {
    out.push('<');
    out.push_str(&n.name);
    for (key, value) in &n.attrs {
        if key.contains('@') {
            continue;
        }
        out.push(' ');
        out.push_str(key);
        out.push_str("=\"");
        out.push_str(&esc(value));
        out.push('"');
    }
    out.push('>');
    for c in &n.children {
        if c.own && allowed(&n.name, &c.name) {
            serialize(c, out);
        }
    }
    out.push_str("</");
    out.push_str(&n.name);
    out.push('>');
}
pub(super) fn parse(xml: &str) -> Result<Collections> {
    let root = document(xml, "styleSheet", MAIN).map_err(bad)?;
    let mut dxfs = String::from("<styleSheet><dxfs>");
    if let Some(nodes) = root.optional("dxfs").map_err(bad)? {
        for n in nodes.children("dxf") {
            serialize(n, &mut dxfs);
        }
    }
    dxfs.push_str("</dxfs></styleSheet>");
    let differential = crate::conditional_format::parse_differential_styles(&dxfs)?;
    let mut tables = Vec::new();
    let mut default_pivot = None;
    if let Some(nodes) = root.optional("tableStyles").map_err(bad)? {
        default_pivot = nodes
            .attr("defaultPivotStyle")
            .map(crate::xstring::decode_xstring);
        for n in nodes.children("tableStyle") {
            let boolean = |name| -> Result<Option<bool>> {
                n.attr(name)
                    .map(|_| n.boolean(name, true).map_err(bad))
                    .transpose()
            };
            tables.push(TableStyleDefinition {
                name: crate::xstring::decode_xstring(n.attr("name").unwrap_or("")),
                table: boolean("table")?,
                pivot: boolean("pivot")?,
                elements: n
                    .children("tableStyleElement")
                    .map(|e| {
                        Ok(TableStyleElement {
                            kind: e.attr("type").unwrap_or("").into(),
                            differential_style_index: e.u32("dxfId").map_err(bad)?,
                            size: e.u32("size").map_err(bad)?.unwrap_or(1).max(1),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
            });
        }
    }
    Ok(Collections {
        differential,
        tables,
        default_pivot,
    })
}
