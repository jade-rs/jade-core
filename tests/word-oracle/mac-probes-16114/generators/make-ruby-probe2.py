#!/usr/bin/env python3
"""Word probe: ruby (w:ruby) geometry, second set: a multi-character base
narrower than its ruby under every alignment, a ruby narrower than a long
base, and odd hpsRaise values (half-point raises).

usage: make-ruby-probe2.py <out-dir>
Each case is a three-line block: a plain line, a line with one ruby in the
middle, and a plain line, so the base widening, the ruby's position, its raise
and the line growth all show. Cases vary the ruby size (hps), raise
(hpsRaise), alignment, base and ruby lengths; ruby2-nogrid and ruby2-grid (word-mac-probe-ruby-wide-*).
"""
import json, os, sys, zipfile
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
CASES = []
for align in ("center", "distributeLetter", "distributeSpace", "left", "right"):
    for base, rt in (("漢字", "かんじかんじ"), ("東京都", "とうきょうと"), ("東京都特許", "とう")):
        CASES.append(dict(align=align, base=base, rt=rt, hps=11, raise_=21))
for raise_ in (19, 23, 25, 27, 17):
    CASES.append(dict(align="center", base="閏", rt="じゆん", hps=11, raise_=raise_))
def ruby(c):
    return (f'<w:r><w:ruby><w:rubyPr><w:rubyAlign w:val="{c["align"]}"/><w:hps w:val="{c["hps"]}"/>'
            f'<w:hpsRaise w:val="{c["raise_"]}"/><w:hpsBaseText w:val="21"/><w:lid w:val="ja-JP"/></w:rubyPr>'
            f'<w:rt><w:r><w:rPr><w:sz w:val="{c["hps"]}"/></w:rPr><w:t>{c["rt"]}</w:t></w:r></w:rt>'
            f'<w:rubyBase><w:r><w:t>{c["base"]}</w:t></w:r></w:rubyBase></w:ruby></w:r>')
def doc(grid):
    body = ""
    for i, c in enumerate(CASES):
        body += f'<w:p><w:r><w:t>前の行{i}あいうえお</w:t></w:r></w:p>'
        body += f'<w:p><w:r><w:t>かきくけこ</w:t></w:r>{ruby(c)}<w:r><w:t>さしすせそ{i}</w:t></w:r></w:p>'
        body += f'<w:p><w:r><w:t>後の行{i}たちつてと</w:t></w:r></w:p>'
    g = '<w:docGrid w:type="lines" w:linePitch="360"/>' if grid else '<w:docGrid w:linePitch="360"/>'
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}'
            '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" '
            f'w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>{g}</w:sectPr></w:body></w:document>')
styles = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}"><w:docDefaults>'
          '<w:rPrDefault><w:rPr><w:rFonts w:ascii="ＭＳ 明朝" w:eastAsia="ＭＳ 明朝" w:hAnsi="ＭＳ 明朝" w:cs="Times New Roman"/>'
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
for name, grid in (("ruby2-nogrid", False), ("ruby2-grid", True)):
    with zipfile.ZipFile(os.path.join(out, name + ".docx"), "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", ct); z.writestr("_rels/.rels", rels)
        z.writestr("word/_rels/document.xml.rels", drels); z.writestr("word/document.xml", doc(grid))
        z.writestr("word/styles.xml", styles); z.writestr("word/settings.xml", settings)
json.dump(CASES, open(os.path.join(out, "manifest.json"), "w"), ensure_ascii=False, indent=1)
