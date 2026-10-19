#!/usr/bin/env python3
"""Word probe: row heights beside a vertically merged cell, and at-least rows.

usage: make-merged-rows-probe.py <out-dir>
Three-column tables in MS PGothic 10.5pt with 4pt cell margins, the first
column merged over every row (yokohama_table's rows 3-5). The merged cell's
continuations carry no w:tcMar unless a variant gives them one. Variants:
the yokohama rows, a thin merged bottom border, no merge, a 3pt merged
bottom border, 8pt of continuation bottom margin, four rows. Then two
header rows of empty paragraphs at least 14.25pt tall over a 1.5pt border,
one under a nil top border and one under a 0.25pt top border.
"""
import os, sys, zipfile
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
FACE = "ＭＳ Ｐゴシック"
MAR = ('<w:tcMar><w:top w:type="dxa" w:w="80"/><w:left w:type="dxa" w:w="80"/>'
       '<w:bottom w:type="dxa" w:w="80"/><w:right w:type="dxa" w:w="80"/></w:tcMar>')
def borders(top, bottom, left=2, right=2):
    side = lambda name, size: (f'<w:{name} w:val="nil"/>' if size is None else
                               f'<w:{name} w:val="single" w:color="000000" w:sz="{size}" w:space="0"/>')
    return f'<w:tcBorders>{side("top", top)}{side("left", left)}{side("bottom", bottom)}{side("right", right)}</w:tcBorders>'
def para(text):
    if not text:
        return "<w:p/>"
    return (f'<w:p><w:pPr><w:jc w:val="left"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="{FACE}" w:hAnsi="{FACE}" '
            f'w:eastAsia="{FACE}"/><w:kern w:val="0"/><w:sz w:val="21"/></w:rPr><w:t>{text}</w:t></w:r></w:p>')
def cell(text, cell_borders, merge=None, margins=MAR):
    vmerge = {"restart": '<w:vMerge w:val="restart"/>', "continue": "<w:vMerge/>"}.get(merge, "")
    return f'<w:tc><w:tcPr><w:tcW w:type="dxa" w:w="2800"/>{vmerge}{cell_borders}{margins}</w:tcPr>{para(text)}</w:tc>'
def row(height, cells):
    tr_pr = f'<w:trPr><w:trHeight w:val="{height}" w:hRule="atLeast"/></w:trPr>' if height else ""
    return f'<w:tr>{tr_pr}{"".join(cells)}</w:tr>'
def table(rows):
    grid = '<w:tblGrid><w:gridCol w:w="2800"/><w:gridCol w:w="2800"/><w:gridCol w:w="2800"/></w:tblGrid>'
    return (f'<w:tbl><w:tblPr><w:tblW w:w="8400" w:type="dxa"/><w:tblInd w:w="108" w:type="dxa"/>'
            f'<w:tblLayout w:type="fixed"/></w:tblPr>{grid}{"".join(rows)}</w:tbl>')
def merged_rows(label, rows=3, merged_bottom=12, merge=True, continuation_margins=""):
    out = []
    for index in range(rows):
        last = index == rows - 1
        bottom = 12 if last else 2
        if merge:
            first = cell(f"{label}結合" if index == 0 else "", borders(2, merged_bottom, 12),
                         "restart" if index == 0 else "continue", MAR if index == 0 else continuation_margins)
        else:
            first = cell(f"{label}左{index}", borders(2, bottom, 12))
        out.append(row((200, 200, 212)[index % 3],
                       [first, cell(f"{label}中{index}", borders(2, bottom)), cell(f"{label}右{index}", borders(2, bottom, 2, 12))]))
    return table(out) + para(f"{label}の後")
def header_rows(label, top):
    header = row(285, [cell("", borders(top, 12, None, None)) for _ in range(3)])
    body = row(None, [cell(f"{label}本文{index}", borders(12, 2)) for index in range(3)])
    return table([header, body]) + para(f"{label}の後")
def doc():
    bottom_margin = '<w:tcMar><w:bottom w:type="dxa" w:w="160"/></w:tcMar>'
    body = (para("はじめ") + merged_rows("Ａ") + merged_rows("Ｂ", merged_bottom=2) + merged_rows("Ｃ", merge=False)
            + merged_rows("Ｄ", merged_bottom=24) + merged_rows("Ｅ", continuation_margins=bottom_margin)
            + merged_rows("Ｆ", rows=4) + header_rows("Ｇ", None) + header_rows("Ｈ", 2))
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}'
            '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="720" w:right="1440" w:bottom="720" '
            'w:left="1440" w:header="360" w:footer="360" w:gutter="0"/></w:sectPr></w:body></w:document>')
styles = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}"><w:docDefaults>'
          '<w:rPrDefault><w:rPr><w:rFonts w:ascii="Century" w:eastAsia="ＭＳ 明朝" w:hAnsi="Century" w:cs="Times New Roman"/>'
          '<w:kern w:val="2"/><w:sz w:val="21"/><w:szCs w:val="24"/><w:lang w:val="en-US" w:eastAsia="ja-JP" w:bidi="ar-SA"/>'
          '</w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults>'
          '<w:style w:type="paragraph" w:default="1" w:styleId="a"><w:name w:val="Normal"/><w:pPr><w:widowControl w:val="0"/>'
          f'<w:jc w:val="both"/></w:pPr><w:rPr><w:rFonts w:ascii="{FACE}" w:hAnsi="{FACE}" w:eastAsia="{FACE}"/></w:rPr></w:style></w:styles>')
settings = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:w="{W}"><w:defaultTabStop w:val="840"/>'
            '<w:compat><w:compatSetting w:name="compatibilityMode" '
            'w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>')
ct = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
      '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
      '<Default Extension="xml" ContentType="application/xml"/>'
      '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
      '<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>'
      '<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>')
rels = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>')
drels = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
         '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>'
         '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/></Relationships>')
out = sys.argv[1]
with zipfile.ZipFile(os.path.join(out, "word-mac-probe-merged-rows.docx"), "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("[Content_Types].xml", ct); z.writestr("_rels/.rels", rels)
    z.writestr("word/_rels/document.xml.rels", drels); z.writestr("word/document.xml", doc())
    z.writestr("word/styles.xml", styles); z.writestr("word/settings.xml", settings)
