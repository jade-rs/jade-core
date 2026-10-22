//! New documents (新規文書): a blank one as Word makes it, and one made from
//! a template (`.dotx`, `.dotm`), as Word opens a template — a new document
//! with the template's text and styles, not the template itself.

use jade_opc::OpcPackage;

/// Which Word's blank document to make: the defaults differ by language.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blank {
    /// 日本語版 Word: A4, 35 mm above and 30 mm on the other sides, 游明朝
    /// 10.5 pt on a 18 pt line grid, justified.
    Japanese,
    /// English Word: Letter, one-inch margins, Aptos 12 pt, 8 pt after each
    /// paragraph at 1.08 lines.
    English,
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/><Default Extension="gif" ContentType="image/gif"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/><Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/></Types>"#;

const PACKAGE_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/></Relationships>"#;

const DOCUMENT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/></Relationships>"#;

const CORE: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title></dc:title><dc:creator></dc:creator></cp:coreProperties>"#;

const APP: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties"><Application>jade office</Application></Properties>"#;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

/// A blank document as `.docx` bytes.
pub fn blank_document(blank: Blank) -> Vec<u8> {
    // Page box and margins in twentieths of a point; the line grid Japanese
    // Word lays a new document on.
    let (page, margins, grid) = match blank {
        Blank::Japanese => (
            r#"<w:pgSz w:w="11906" w:h="16838"/>"#,
            r#"<w:pgMar w:top="1985" w:right="1701" w:bottom="1701" w:left="1701" w:header="851" w:footer="992" w:gutter="0"/>"#,
            r#"<w:cols w:space="425"/><w:docGrid w:type="lines" w:linePitch="360"/>"#,
        ),
        Blank::English => (
            r#"<w:pgSz w:w="12240" w:h="15840"/>"#,
            r#"<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>"#,
            r#"<w:cols w:space="720"/><w:docGrid w:linePitch="360"/>"#,
        ),
    };
    let document = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="{W}" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body><w:p/><w:sectPr>{page}{margins}{grid}</w:sectPr></w:body></w:document>"#
    );
    let (fonts, size, paragraph, language) = match blank {
        Blank::Japanese => (
            r#"<w:rFonts w:ascii="游明朝" w:eastAsia="游明朝" w:hAnsi="游明朝" w:cs="Times New Roman"/>"#,
            21,
            r#"<w:jc w:val="both"/>"#,
            r#"<w:lang w:val="en-US" w:eastAsia="ja-JP" w:bidi="ar-SA"/>"#,
        ),
        Blank::English => (
            r#"<w:rFonts w:ascii="Aptos" w:eastAsia="Aptos" w:hAnsi="Aptos" w:cs="Times New Roman"/>"#,
            24,
            r#"<w:spacing w:after="160" w:line="278" w:lineRule="auto"/>"#,
            r#"<w:lang w:val="en-US" w:eastAsia="en-US" w:bidi="ar-SA"/>"#,
        ),
    };
    let kern = match blank {
        Blank::Japanese => r#"<w:kern w:val="2"/>"#,
        Blank::English => "",
    };
    let styles = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="{W}"><w:docDefaults><w:rPrDefault><w:rPr>{fonts}{kern}<w:sz w:val="{size}"/><w:szCs w:val="{size}"/>{language}</w:rPr></w:rPrDefault><w:pPrDefault><w:pPr>{paragraph}</w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>{headings}<w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont"><w:name w:val="Default Paragraph Font"/><w:uiPriority w:val="1"/><w:semiHidden/><w:unhideWhenUsed/></w:style><w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style><w:style w:type="numbering" w:default="1" w:styleId="NoList"><w:name w:val="No List"/><w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/></w:style></w:styles>"#,
        headings = headings(blank),
    );
    let settings = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:settings xmlns:w="{W}"><w:zoom w:percent="100"/><w:defaultTabStop w:val="{tab}"/>{spacing}<w:characterSpacingControl w:val="{control}"/><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>"#,
        tab = match blank {
            Blank::Japanese => 840,
            Blank::English => 720,
        },
        spacing = match blank {
            Blank::Japanese =>
                r#"<w:drawingGridVerticalSpacing w:val="180"/><w:displayHorizontalDrawingGridEvery w:val="0"/><w:displayVerticalDrawingGridEvery w:val="2"/>"#,
            Blank::English => "",
        },
        control = match blank {
            Blank::Japanese => "compressPunctuation",
            Blank::English => "doNotCompress",
        },
    );
    let parts: [(&str, &str); 7] = [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", PACKAGE_RELS),
        ("word/document.xml", &document),
        ("word/styles.xml", &styles),
        ("word/settings.xml", &settings),
        ("word/_rels/document.xml.rels", DOCUMENT_RELS),
        ("docProps/core.xml", CORE),
    ];
    let mut package = OpcPackage::open(EMPTY_ZIP).expect("an empty archive opens");
    for (name, xml) in parts {
        package.replace(name, xml.as_bytes().to_vec());
    }
    package.replace("docProps/app.xml", APP.as_bytes().to_vec());
    package.write().expect("a package in memory writes")
}

/// Heading 1–3 as Word defines them, so the gallery's 見出し apply as Word's.
fn headings(blank: Blank) -> String {
    let sizes: [u32; 3] = match blank {
        Blank::Japanese => [24, 22, 21],
        Blank::English => [40, 32, 28],
    };
    let fonts = match blank {
        // The theme's heading face, named: a blank document carries no theme.
        Blank::Japanese => {
            r#"<w:rFonts w:ascii="游ゴシック Light" w:eastAsia="游ゴシック Light" w:hAnsi="游ゴシック Light"/>"#
        }
        Blank::English => "",
    };
    (1..=3)
        .map(|level| {
            let size = sizes[level - 1];
            format!(
                r#"<w:style w:type="paragraph" w:styleId="Heading{level}"><w:name w:val="heading {level}"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="9"/><w:qFormat/><w:pPr><w:keepNext/><w:outlineLvl w:val="{outline}"/></w:pPr><w:rPr>{fonts}<w:sz w:val="{size}"/><w:szCs w:val="{size}"/></w:rPr></w:style>"#,
                outline = level - 1,
            )
        })
        .collect()
}

/// A ZIP archive with nothing in it: just the end of its central directory.
const EMPTY_ZIP: &[u8] = &[
    0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

const TEMPLATE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml";
const DOCUMENT: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml";
const MACRO_TEMPLATE: &str = "application/vnd.ms-word.template.macroEnabledTemplate.main+xml";
const MACRO_DOCUMENT: &str = "application/vnd.ms-word.document.macroEnabled.main+xml";

/// A new document from a template's bytes: the template's main part
/// declared a document's, so it saves as `.docx` (or `.docm` for a
/// macro-enabled template — see [`is_macro_enabled`]). A document given
/// here comes back as it is.
pub fn document_from_template(template: &[u8]) -> Result<Vec<u8>, String> {
    let mut package =
        OpcPackage::open(template).map_err(|error| format!("cannot read the template: {error}"))?;
    let types = package
        .part_str("[Content_Types].xml")
        .map_err(|error| format!("cannot read the template: {error}"))?
        .ok_or("the template has no content types")?;
    let declared = types
        .replace(TEMPLATE, DOCUMENT)
        .replace(MACRO_TEMPLATE, MACRO_DOCUMENT);
    if declared == types {
        return Ok(template.to_vec());
    }
    package.replace("[Content_Types].xml", declared.into_bytes());
    package
        .write()
        .map_err(|error| format!("cannot write the document: {error}"))
}

/// Whether a template keeps macros, so a document made from it is a
/// `.docm`.
pub fn is_macro_enabled(template: &[u8]) -> bool {
    OpcPackage::open(template)
        .ok()
        .and_then(|package| package.part_str("[Content_Types].xml").ok().flatten())
        .is_some_and(|types| types.contains(MACRO_TEMPLATE) || types.contains(MACRO_DOCUMENT))
}

/// Whether `name` is a Word template, which opens as a new document.
pub fn is_template_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".dotx") || name.ends_with(".dotm")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_document_is_a_package_word_reads() {
        for blank in [Blank::Japanese, Blank::English] {
            let bytes = blank_document(blank);
            let package = OpcPackage::open(&bytes).unwrap();
            let document = package.part_str("word/document.xml").unwrap().unwrap();
            assert!(document.contains("<w:body><w:p/>"));
            let styles = package.part_str("word/styles.xml").unwrap().unwrap();
            assert!(styles.contains(r#"w:styleId="Heading1""#));
        }
        let japanese = blank_document(Blank::Japanese);
        let document = OpcPackage::open(&japanese)
            .unwrap()
            .part_str("word/document.xml")
            .unwrap()
            .unwrap();
        assert!(document.contains(r#"w:w="11906""#), "A4");
        assert!(document.contains(r#"w:top="1985""#), "35 mm");
    }

    #[test]
    fn a_template_opens_as_a_document() {
        let blank = blank_document(Blank::English);
        let mut template = OpcPackage::open(&blank).unwrap();
        let types = template.part_str("[Content_Types].xml").unwrap().unwrap();
        template.replace(
            "[Content_Types].xml",
            types.replace(DOCUMENT, TEMPLATE).into_bytes(),
        );
        let template = template.write().unwrap();
        assert!(!is_macro_enabled(&template));

        let document = document_from_template(&template).unwrap();
        let types = OpcPackage::open(&document)
            .unwrap()
            .part_str("[Content_Types].xml")
            .unwrap()
            .unwrap();
        assert!(types.contains(DOCUMENT));
        assert!(!types.contains(TEMPLATE));
        // A document is left as it is.
        assert_eq!(document_from_template(&blank).unwrap(), blank);
        assert!(is_template_name("Report.DOTX"));
        assert!(!is_template_name("report.docx"));
    }
}
