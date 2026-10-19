#!/usr/bin/env python3
"""Word probe: line pitch of auto line spacing in Latin faces.

usage: make-line-pitch-probe.py <out-dir>
Each block is one paragraph of eight lines (manual breaks) in one face and
size at `w:line` 240, 276 or 360 auto, then an empty paragraph, so the
baseline steps give Word's line height for the face. Faces: Arial, Calibri,
Times New Roman, Century; 10, 11 and 12pt. No grid, compatibility mode 15.
"""
import os, sys, zipfile
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
FACES = ["Arial", "Calibri", "Times New Roman", "Century"]
SIZES = [20, 22, 24]
LINES = [240, 276, 360]
def doc():
    body = ""
    for line in LINES:
        for face in FACES:
            for size in SIZES:
                rpr = (f'<w:rPr><w:rFonts w:ascii="{face}" w:hAnsi="{face}" w:eastAsia="{face}"/>'
                       f'<w:sz w:val="{size}"/><w:szCs w:val="{size}"/></w:rPr>')
                runs = "".join(f'<w:r>{rpr}<w:t>Line {i} {face} {size / 2:g}pt {line}</w:t></w:r>'
                               + (f'<w:r>{rpr}<w:br/></w:r>' if i < 8 else "") for i in range(1, 9))
                body += (f'<w:p><w:pPr><w:spacing w:before="0" w:after="0" w:line="{line}" w:lineRule="auto"/>'
                         f'</w:pPr>{runs}</w:p>')
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
with zipfile.ZipFile(os.path.join(out, "word-mac-probe-line-pitch.docx"), "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("[Content_Types].xml", ct); z.writestr("_rels/.rels", rels)
    z.writestr("word/_rels/document.xml.rels", drels); z.writestr("word/document.xml", doc())
    z.writestr("word/styles.xml", styles); z.writestr("word/settings.xml", settings)
