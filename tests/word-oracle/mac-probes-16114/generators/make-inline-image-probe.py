#!/usr/bin/env python3
"""Word probe: spacing around an inline picture.

usage: make-inline-image-probe.py <out-dir>
Each paragraph puts a 20pt inline picture between two characters (kana,
Latin, digit), so the gaps Word leaves on either side of the picture show.
The same cases repeat with w:autoSpaceDE/w:autoSpaceDN off. Century for
Latin, MS Mincho for East Asian text; no grid (inlineimg-nogrid) and a line
grid (inlineimg-grid).
"""
import os, struct, sys, zipfile, zlib
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
PAIRS = [("あ", "あ"), ("あ", "W"), ("W", "あ"), ("W", "W"), ("あ", "1"), ("1", "あ"), ("1", "1"), ("W ", " W")]
def png():
    raw = b"".join(b"\x00" + b"\xff\x00\x00" * 4 for _ in range(4))
    def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 4, 4, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))
EMU = 254000  # 20pt
def pic(i):
    return (f'<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0" '
            'xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing">'
            f'<wp:extent cx="{EMU}" cy="{EMU}"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:docPr id="{i}" name="p{i}"/>'
            '<a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">'
            '<pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:nvPicPr><pic:cNvPr id="0" name="p.png"/><pic:cNvPicPr/></pic:nvPicPr>'
            '<pic:blipFill><a:blip r:embed="rIdImg" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>'
            f'<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{EMU}" cy="{EMU}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>'
            '</a:graphicData></a:graphic></wp:inline></w:drawing></w:r>')
def t(text): return f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>'
def doc(grid):
    body, n = "", 1
    for auto in (True, False):
        for before, after in PAIRS:
            ppr = "" if auto else '<w:pPr><w:autoSpaceDE w:val="0"/><w:autoSpaceDN w:val="0"/></w:pPr>'
            body += f'<w:p>{ppr}{t("前" + before)}{pic(n)}{t(after + "後")}</w:p>'
            n += 1
    g = '<w:docGrid w:type="lines" w:linePitch="360"/>' if grid else '<w:docGrid w:linePitch="360"/>'
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}'
            '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" '
            f'w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>{g}</w:sectPr></w:body></w:document>')
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
      '<Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/>'
      '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
      '<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>'
      '<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/></Types>')
rels = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>')
drels = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
         '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>'
         '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/>'
         '<Relationship Id="rIdImg" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/p.png"/></Relationships>')
out = sys.argv[1]
for name, grid in (("inlineimg-nogrid", False), ("inlineimg-grid", True)):
    with zipfile.ZipFile(os.path.join(out, name + ".docx"), "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", ct); z.writestr("_rels/.rels", rels)
        z.writestr("word/_rels/document.xml.rels", drels); z.writestr("word/document.xml", doc(grid))
        z.writestr("word/styles.xml", styles); z.writestr("word/settings.xml", settings)
        z.writestr("word/media/p.png", png())
