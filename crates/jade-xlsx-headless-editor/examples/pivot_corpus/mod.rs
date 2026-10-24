//! Original Strict namespace/lexical fixture conversion shared by QA examples.
pub fn strict_original(original: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut package = jade_xlsx_reader::OpcPackage::open(original)?;
    for part in package
        .part_names()
        .to_vec()
        .into_iter()
        .filter(|p| p.ends_with(".xml") || p.ends_with(".rels"))
    {
        let mut xml = package.required_part_str(&part)?;
        for (from, to) in [
            (
                "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
                "http://purl.oclc.org/ooxml/spreadsheetml/main",
            ),
            (
                "http://schemas.openxmlformats.org/drawingml/2006/main",
                "http://purl.oclc.org/ooxml/drawingml/main",
            ),
            (
                "http://schemas.openxmlformats.org/drawingml/2006/chart",
                "http://purl.oclc.org/ooxml/drawingml/chart",
            ),
            (
                "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing",
                "http://purl.oclc.org/ooxml/drawingml/spreadsheetDrawing",
            ),
            (
                "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
                "http://purl.oclc.org/ooxml/officeDocument/relationships",
            ),
        ] {
            xml = xml.replace(from, to);
        }
        // The Strict lexical type permits a percentage, not an unadorned integer.
        xml = xml.replace("<c:gapWidth val=\"150\"/>", "<c:gapWidth val=\"150%\"/>");
        package.replace(&part, xml.into_bytes());
    }
    Ok(package.write()?)
}
