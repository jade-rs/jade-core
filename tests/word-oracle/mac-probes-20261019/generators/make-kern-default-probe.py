#!/usr/bin/env python3
"""Word probe: which pair kerning Word for Mac applies without w:kern.

usage: make-kern-default-probe.py <out-dir>
Each paragraph sets pairs from the faces' kern tables — letter pairs (`AV`,
`To`), digit pairs (`11`) and pairs with a space (` A`, `A `, `L `, ` T`) —
in Arial, Times New Roman, Century, Calibri and Cambria at 20pt, once
without `w:kern` and once with `w:kern w:val="1"`, so the glyph positions
show which pairs Word closes up by default.
"""
import os, sys, zipfile
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
FACES = ["Arial", "Times New Roman", "Century", "Calibri", "Cambria"]
TEXTS = ["xAVx", "xTox", "x11x", "x Ax", "xA x", "xL x", "x Tx", "xWa.x"]
def doc():
    body = ""
    for kern in (False, True):
        for face in FACES:
            k = '<w:kern w:val="1"/>' if kern else ""
            rpr = f'<w:rPr><w:rFonts w:ascii="{face}" w:hAnsi="{face}" w:eastAsia="{face}"/>{k}<w:sz w:val="40"/></w:rPr>'
            runs = "".join(f'<w:r>{rpr}<w:t xml:space="preserve">{text}</w:t></w:r>' + (f'<w:r>{rpr}<w:tab/></w:r>' if i < len(TEXTS) - 1 else "")
                           for i, text in enumerate(TEXTS))
            body += f'<w:p>{runs}</w:p>'
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}'
            '<w:sectPr><w:pgSz w:w="16838" w:h="11906" w:orient="landscape"/><w:pgMar w:top="720" w:right="720" w:bottom="720" '
            'w:left="720" w:header="360" w:footer="360" w:gutter="0"/><w:docGrid w:linePitch="360"/>'
            '</w:sectPr></w:body></w:document>')
styles = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}"><w:docDefaults>'
          '<w:rPrDefault><w:rPr><w:rFonts w:ascii="Century" w:eastAsia="ＭＳ 明朝" w:hAnsi="Century" w:cs="Times New Roman"/>'
          '<w:kern w:val="2"/><w:sz w:val="21"/><w:szCs w:val="24"/><w:lang w:val="en-US" w:eastAsia="ja-JP" w:bidi="ar-SA"/>'
          '</w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults>'
          '<w:style w:type="paragraph" w:default="1" w:styleId="a"><w:name w:val="Normal"/><w:pPr><w:widowControl w:val="0"/>'
          '<w:jc w:val="left"/></w:pPr></w:style></w:styles>')
settings = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:w="{W}"><w:defaultTabStop w:val="1440"/>'
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
with zipfile.ZipFile(os.path.join(out, "word-mac-probe-kern-default.docx"), "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("[Content_Types].xml", ct); z.writestr("_rels/.rels", rels)
    z.writestr("word/_rels/document.xml.rels", drels); z.writestr("word/document.xml", doc())
    z.writestr("word/styles.xml", styles); z.writestr("word/settings.xml", settings)
