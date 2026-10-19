#!/usr/bin/env python3
"""Word probe: the size of superscript and subscript text.

usage: make-vert-align-probe.py <out-dir>
Each paragraph is `A12B` with `12` raised (or lowered) in one face and size,
so the advance of the digits and the gap before `B` give the size Word draws
the shifted run at. Faces: Yu Mincho, Century, MS Mincho, Calibri; sizes
8 to 20pt. No grid, left aligned, compatibility mode 15.
"""
import os, sys, zipfile
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
FACES = [("游明朝", "游明朝"), ("Century", "ＭＳ 明朝"), ("ＭＳ 明朝", "ＭＳ 明朝"), ("Calibri", "ＭＳ 明朝")]
SIZES = [16, 18, 21, 24, 28, 32, 40]  # half-points: 8 to 20pt
def run(text, latin, east, size, align=None):
    va = f'<w:vertAlign w:val="{align}"/>' if align else ""
    return (f'<w:r><w:rPr><w:rFonts w:ascii="{latin}" w:hAnsi="{latin}" w:eastAsia="{east}"/>'
            f'{va}<w:sz w:val="{size}"/><w:szCs w:val="{size}"/></w:rPr>'
            f'<w:t xml:space="preserve">{text}</w:t></w:r>')
def doc():
    body = ""
    for align in ("superscript", "subscript"):
        for latin, east in FACES:
            for size in SIZES:
                body += ("<w:p>" + run("A", latin, east, size) + run("12", latin, east, size, align)
                         + run("B", latin, east, size) + "</w:p>")
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}'
            '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="720" w:right="1440" w:bottom="720" '
            'w:left="1440" w:header="360" w:footer="360" w:gutter="0"/><w:docGrid w:linePitch="360"/>'
            '</w:sectPr></w:body></w:document>')
styles = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}"><w:docDefaults>'
          '<w:rPrDefault><w:rPr><w:rFonts w:ascii="Century" w:eastAsia="ＭＳ 明朝" w:hAnsi="Century" w:cs="Times New Roman"/>'
          '<w:kern w:val="2"/><w:sz w:val="21"/><w:szCs w:val="24"/><w:lang w:val="en-US" w:eastAsia="ja-JP" w:bidi="ar-SA"/>'
          '</w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults>'
          '<w:style w:type="paragraph" w:default="1" w:styleId="a"><w:name w:val="Normal"/><w:pPr><w:widowControl w:val="0"/>'
          '<w:jc w:val="left"/></w:pPr></w:style></w:styles>')
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
with zipfile.ZipFile(os.path.join(out, "word-mac-probe-vert-align.docx"), "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("[Content_Types].xml", ct); z.writestr("_rels/.rels", rels)
    z.writestr("word/_rels/document.xml.rels", drels); z.writestr("word/document.xml", doc())
    z.writestr("word/styles.xml", styles); z.writestr("word/settings.xml", settings)
