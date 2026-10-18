#!/usr/bin/env python3
"""Word probe: ruby written as an EQ overstrike field (EQ \\* jcN \\* hpsN
\\o\\aX(\\s\\up N(guide),base)) with an empty saved result, which Word
computes itself.

usage: make-eqruby-probe.py <out-dir>
Each case is a three-line block: a plain line, a line with one ruby in the
middle, and a plain line, so the base widening, the ruby's position, its raise
and the line growth all show. Cases vary the ruby size (hps), raise
(hpsRaise), alignment, base and ruby lengths; eqruby-nogrid and eqruby-grid.
"""
import json, os, sys, zipfile
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
CASES = []
for jc, sw in ((0, "\\ac"), (1, "\\ad"), (2, "\\ad"), (3, "\\al"), (4, "\\ar")):
    for base, rt in (("閏", "じゆん"), ("漢字", "かんじかんじ"), ("東京都特許", "とう")):
        CASES.append(dict(jc=jc, sw=sw, base=base, rt=rt, hps=11, up=10))
for up in (9, 11, 15):
    CASES.append(dict(jc=2, sw="\\ad", base="閏", rt="じゆん", hps=11, up=up))
def ruby(c):
    instr = (f' EQ \\* jc{c["jc"]} \\* "Font:ＭＳ 明朝" \\* hps{c["hps"]} '
             f'\\o{c["sw"]}(\\s\\up {c["up"]}({c["rt"]}),{c["base"]})')
    return ('<w:r><w:fldChar w:fldCharType="begin"/></w:r>'
            f'<w:r><w:instrText xml:space="preserve">{instr}</w:instrText></w:r>'
            '<w:r><w:fldChar w:fldCharType="separate"/></w:r>'
            '<w:r><w:fldChar w:fldCharType="end"/></w:r>')
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
for name, grid in (("eqruby-nogrid", False), ("eqruby-grid", True)):
    with zipfile.ZipFile(os.path.join(out, name + ".docx"), "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", ct); z.writestr("_rels/.rels", rels)
        z.writestr("word/_rels/document.xml.rels", drels); z.writestr("word/document.xml", doc(grid))
        z.writestr("word/styles.xml", styles); z.writestr("word/settings.xml", settings)
json.dump(CASES, open(os.path.join(out, "manifest.json"), "w"), ensure_ascii=False, indent=1)
