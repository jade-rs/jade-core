#!/usr/bin/env python3
"""Writes the Word for Mac probes that isolate word-mac-grid-tall-face's rules.

usage: make_grid_tall_face_probes.py <out-dir>
"""
import os
import sys
import zipfile

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
NS = f'xmlns:w="{W}"'
PAGE = (
    '<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1985" w:right="1701" w:bottom="1701" '
    'w:left="1701" w:header="851" w:footer="992" w:gutter="0"/>'
)
LINE_GRID = '<w:docGrid w:type="lines" w:linePitch="360"/>'


def package(path, body, compat=""):
    styles = (
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles {NS}><w:docDefaults>'
        '<w:rPrDefault><w:rPr><w:rFonts w:ascii="Times New Roman" w:eastAsia="ＭＳ 明朝" w:hAnsi="ＭＳ 明朝"/>'
        '<w:sz w:val="21"/><w:szCs w:val="21"/></w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults>'
        '<w:style w:type="paragraph" w:default="1" w:styleId="a"><w:name w:val="Normal"/><w:qFormat/></w:style>'
        '<w:style w:type="table" w:default="1" w:styleId="a1"><w:name w:val="Normal Table"/><w:semiHidden/><w:tblPr>'
        '<w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/>'
        '<w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style></w:styles>'
    )
    settings = (
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings {NS}><w:defaultTabStop w:val="720"/>'
        f'<w:compat>{compat}<w:useFELayout/><w:compatSetting w:name="compatibilityMode" '
        'w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>'
    )
    document = (
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{body}</w:body></w:document>'
    )
    types = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
        '<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>'
        '<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>'
    )
    rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>'
    )
    document_rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>'
        '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/></Relationships>'
    )
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as package_zip:
        for name, data in (
            ("[Content_Types].xml", types),
            ("_rels/.rels", rels),
            ("word/_rels/document.xml.rels", document_rels),
            ("word/document.xml", document),
            ("word/styles.xml", styles),
            ("word/settings.xml", settings),
        ):
            info = zipfile.ZipInfo(name, date_time=(2026, 11, 7, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            package_zip.writestr(info, data)


def exact_line(text, spacing, ppr=""):
    return (
        f'<w:p><w:pPr>{ppr}<w:spacing {spacing} w:line="240" w:lineRule="exact"/></w:pPr>'
        f'<w:r><w:t>{text}</w:t></w:r></w:p>'
    )


def grid_line_unit():
    """Each case is a probe between references with exact 12pt lines, so a baseline gap minus 12pt
    is the spacing Word applied."""
    grids = [
        LINE_GRID,
        '<w:docGrid w:type="lines" w:linePitch="300"/>',
        '<w:docGrid w:type="linesAndChars" w:linePitch="360" w:charSpace="0"/>',
        '<w:docGrid w:type="default" w:linePitch="360"/>',
        '<w:docGrid w:linePitch="360"/>',
        "",
    ]
    zero = 'w:before="0" w:after="0"'
    cases = [
        ('w:before="0" w:after="90" w:afterLines="50"', ""),
        ('w:before="0" w:after="80" w:afterLines="44"', ""),
        ('w:before="90" w:beforeLines="50" w:after="0"', ""),
        ('w:before="0" w:after="90" w:afterLines="50"', '<w:snapToGrid w:val="0"/>'),
    ]
    body = ""
    for index, grid in enumerate(grids):
        body += exact_line("基準", zero)
        for spacing, ppr in cases:
            body += exact_line("試験", spacing, ppr) + exact_line("基準", zero)
        section = f"<w:sectPr>{PAGE}{grid}</w:sectPr>"
        if index + 1 < len(grids):
            body += f'<w:p><w:pPr><w:spacing {zero} w:line="240" w:lineRule="exact"/>{section}</w:pPr></w:p>'
        else:
            body += section
    return body


def grid_line_unit_table():
    """A cell's auto lines ignore the grid without w:adjustLineHeightInTable; its line unit does not.
    The third row's gap holds the unit, 9pt here and 6pt in the second section."""
    zero = 'w:before="0" w:after="0"'
    rows = "".join(
        f'<w:p><w:pPr><w:spacing {spacing}/></w:pPr><w:r><w:t>{text}</w:t></w:r></w:p>'
        for text, spacing in (
            ("基準", zero),
            ("試験", 'w:before="0" w:after="90" w:afterLines="50"'),
            ("基準", zero),
        )
    )
    table = (
        '<w:tbl><w:tblPr><w:tblW w:w="4000" w:type="dxa"/></w:tblPr><w:tblGrid><w:gridCol w:w="4000"/></w:tblGrid>'
        f'<w:tr><w:tc><w:tcPr><w:tcW w:w="4000" w:type="dxa"/></w:tcPr>{rows}</w:tc></w:tr></w:tbl>'
    )
    return (
        f'{table}<w:p><w:pPr><w:spacing {zero}/><w:sectPr>{PAGE}{LINE_GRID}</w:sectPr></w:pPr></w:p>'
        f"{table}<w:p/><w:sectPr>{PAGE}</w:sectPr>"
    )


def latin_east_asia_face():
    """Kanji under an eastAsia face without CJK glyphs, on auto lines with no grid: the advance and
    the line box show which face Word set them in."""
    body = ""
    for face in (
        "Arial",
        "Times New Roman",
        "Calibri",
        "Century",
        "Courier New",
        "Verdana",
        "Helvetica",
        "Noto Sans",
        "Arial Unicode MS",
    ):
        for bold in ("", "<w:b/>"):
            body += (
                '<w:p><w:pPr><w:spacing w:before="0" w:after="0"/></w:pPr><w:r><w:rPr>'
                f'<w:rFonts w:ascii="Times New Roman" w:eastAsia="{face}" w:hAnsi="Times New Roman"/>{bold}'
                '<w:sz w:val="24"/></w:rPr><w:t>著作権法の規定による</w:t></w:r></w:p>'
            )
    return body + f"<w:sectPr>{PAGE}</w:sectPr>"


def kinsoku_widow():
    """The line-breaking and pagination rules a 12pt bold Arial Unicode MS statute needs on an 18pt
    line grid, where every line takes two grid steps and 34 characters fill the 425.2pt measure."""

    def paragraph(text, ind=""):
        return (
            f'<w:p><w:pPr>{ind}<w:jc w:val="left"/></w:pPr><w:r><w:rPr>'
            '<w:rFonts w:ascii="Arial Unicode MS" w:eastAsia="Arial Unicode MS" w:hAnsi="Arial Unicode MS"/><w:b/>'
            f'<w:sz w:val="24"/></w:rPr><w:t>{text}</w:t></w:r></w:p>'
        )

    # A comma that does not fit pulls the character before it down, the line it lands on overflows,
    # and the next line takes that overflow.
    cascade = paragraph(
        "公表された著作物（映画の著作物を除く。）は、営利を目的とせず、かつ、その複製物の貸与を受ける者から"
        "料金を受けない場合には、その複製物（映画の著作物において複製されている著作物にあつては、当該映画の"
        "著作物の複製物を除く。）の貸与により公衆に提供することができる。",
        '<w:ind w:left="689" w:firstLine="245"/>',
    )
    # The same repair overfills the paragraph's last line.
    last_line = paragraph(
        "著作権は、これを目的として質権を設定した場合においても、設定行為に別段の定めがない限り、著作権者が行使するものとする。",
        '<w:ind w:left="479" w:firstLine="1228"/>',
    )
    # 18 lines fill a page: after 10 lines on this page and 18 on the next, this paragraph has one
    # line left, so the line before it moves along.
    long_paragraph = paragraph("権" * (34 * 28 + 3))
    return cascade + last_line + long_paragraph + f"<w:sectPr>{PAGE}{LINE_GRID}</w:sectPr>"


def main():
    out = sys.argv[1]
    package(os.path.join(out, "word-mac-probe-grid-line-unit.docx"), grid_line_unit())
    package(os.path.join(out, "word-mac-probe-grid-line-unit-table.docx"), grid_line_unit_table())
    package(os.path.join(out, "word-mac-probe-latin-east-asia-face.docx"), latin_east_asia_face())
    package(os.path.join(out, "word-mac-probe-kinsoku-widow.docx"), kinsoku_widow())


if __name__ == "__main__":
    main()
