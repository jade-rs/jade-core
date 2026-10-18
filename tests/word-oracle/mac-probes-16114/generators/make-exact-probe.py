#!/usr/bin/env python3
"""Word probe: where an exact-height line puts its baseline.

usage: make-exact-probe.py <out-dir>
Each case is one paragraph on its own page (pageBreakBefore), so the first
baseline minus the top margin is the baseline's offset in the exact line.
Writes exact-nogrid.docx and exact-grid.docx (linesAndChars? no: lines 18pt).
"""
import json, os, sys, zipfile
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
CASES = []
for exact in (240, 350, 480, 720):           # twips: 12, 17.5, 24, 36pt
    for font, size in (("ＭＳ 明朝", 21), ("ＭＳ ゴシック", 22), ("ＭＳ 明朝", 40), ("Century", 21)):
        CASES.append((exact, font, size))
def doc(grid):
    body = ""
    for i, (exact, font, size) in enumerate(CASES):
        pb = "<w:pageBreakBefore/>" if i else ""
        body += (f'<w:p><w:pPr>{pb}<w:spacing w:before="0" w:after="0" w:line="{exact}" w:lineRule="exact"/></w:pPr>'
                 f'<w:r><w:rPr><w:rFonts w:ascii="{font}" w:eastAsia="{font}" w:hAnsi="{font}"/><w:sz w:val="{size}"/></w:rPr>'
                 f'<w:t>あ亜Ag{i}</w:t></w:r></w:p>')
    g = '<w:docGrid w:type="lines" w:linePitch="360"/>' if grid else '<w:docGrid w:linePitch="360"/>'
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}'
            '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" '
            f'w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>{g}</w:sectPr></w:body></w:document>')
styles = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}"><w:docDefaults>'
          '<w:rPrDefault><w:rPr><w:rFonts w:ascii="Century" w:eastAsia="ＭＳ 明朝" w:hAnsi="Century" w:cs="Times New Roman"/>'
          '<w:kern w:val="2"/><w:sz w:val="21"/><w:szCs w:val="24"/><w:lang w:val="en-US" w:eastAsia="ja-JP" w:bidi="ar-SA"/>'
          '</w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults>'
          '<w:style w:type="paragraph" w:default="1" w:styleId="a"><w:name w:val="Normal"/><w:pPr><w:widowControl w:val="0"/>'
          '<w:jc w:val="both"/></w:pPr></w:style></w:styles>')
settings = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:w="{W}"><w:defaultTabStop w:val="840"/>'
            '<w:characterSpacingControl w:val="compressPunctuation"/><w:compat><w:compatSetting w:name="compatibilityMode" '
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
for name, grid in (("exact-nogrid", False), ("exact-grid", True)):
    with zipfile.ZipFile(os.path.join(out, name + ".docx"), "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", ct); z.writestr("_rels/.rels", rels)
        z.writestr("word/_rels/document.xml.rels", drels); z.writestr("word/document.xml", doc(grid))
        z.writestr("word/styles.xml", styles); z.writestr("word/settings.xml", settings)
json.dump([{"exact_twips": e, "font": f, "half_points": s} for e, f, s in CASES],
          open(os.path.join(out, "manifest.json"), "w"), ensure_ascii=False, indent=1)
