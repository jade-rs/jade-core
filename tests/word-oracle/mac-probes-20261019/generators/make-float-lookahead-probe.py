#!/usr/bin/env python3
"""Word probe: text before a floating shape's anchor paragraph.

usage: make-float-lookahead-probe.py <out-dir>
Each page holds a short paragraph ("before"), then the anchor paragraph
with a 144 x 72pt rectangle, square wrap on both sides, then body text.
Pages 1-4: the shape sits 0pt below its paragraph at the right margin and
"before" has no spacing, 12pt space after, a bottom border, and 6pt space
after with the shape 24pt above its paragraph. Page 5: the shape sits at
page position (300pt, 90pt), overlapping "before". Arial 11pt, mode 15.
"""
import os, sys, zipfile
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
NS = ('xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" '
      'xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" '
      'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
      'xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"')
EMU = 12700
def para(text, ppr="", run_prefix=""):
    return f'<w:p><w:pPr>{ppr}</w:pPr>{run_prefix}<w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>'
def shape(ident, h_rel, h_off, v_rel, v_off):
    return (f'<w:r><w:drawing><wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" '
            f'relativeHeight="{251659264 + ident}" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1">'
            '<wp:simplePos x="0" y="0"/>'
            f'<wp:positionH relativeFrom="{h_rel}"><wp:posOffset>{int(h_off * EMU)}</wp:posOffset></wp:positionH>'
            f'<wp:positionV relativeFrom="{v_rel}"><wp:posOffset>{int(v_off * EMU)}</wp:posOffset></wp:positionV>'
            f'<wp:extent cx="{144 * EMU}" cy="{72 * EMU}"/><wp:wrapSquare wrapText="bothSides"/>'
            f'<wp:docPr id="{ident}" name="Shape {ident}"/><wp:cNvGraphicFramePr/>'
            '<a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape">'
            f'<wps:wsp><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{144 * EMU}" cy="{72 * EMU}"/></a:xfrm>'
            '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="D9EAD3"/></a:solidFill></wps:spPr>'
            '<wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>')
BODY = ("Body text after the anchor wraps beside the shape while the two overlap and returns to the "
        "full width below it. ") * 3
BREAK = '<w:p><w:r><w:br w:type="page"/></w:r></w:p>'
def case(label, ident, before_ppr, h, v):
    before = ("Before " + label + ": this paragraph comes before the anchor and is long enough to reach "
              "the right margin of the page so that any wrap shows.")
    return (para(before, before_ppr) + para(f"Anchor {label}. " + BODY, "", shape(ident, *h, *v)))
def doc():
    right = ("margin", 324)
    pages = [
        case("A", 1, "", right, ("paragraph", 0)),
        case("B", 2, '<w:spacing w:after="240"/>', right, ("paragraph", 0)),
        case("C", 3, '<w:pBdr><w:bottom w:val="single" w:sz="8" w:space="4" w:color="4F81BD"/></w:pBdr>',
             right, ("paragraph", 0)),
        case("D", 4, '<w:spacing w:after="120"/>', right, ("paragraph", -24)),
        case("E", 5, "", ("page", 300), ("page", 90)),
    ]
    body = BREAK.join(pages)
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{body}'
            '<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" '
            'w:left="1440" w:header="720" w:footer="720" w:gutter="0"/></w:sectPr></w:body></w:document>')
styles = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}"><w:docDefaults>'
          '<w:rPrDefault><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="Arial" w:cs="Arial"/>'
          '<w:sz w:val="22"/><w:lang w:val="en-US"/></w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults>'
          '<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style></w:styles>')
settings = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:w="{W}"><w:defaultTabStop w:val="720"/>'
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
with zipfile.ZipFile(os.path.join(out, "word-mac-probe-float-lookahead.docx"), "w", zipfile.ZIP_DEFLATED) as z:
    z.writestr("[Content_Types].xml", ct); z.writestr("_rels/.rels", rels)
    z.writestr("word/_rels/document.xml.rels", drels); z.writestr("word/document.xml", doc())
    z.writestr("word/styles.xml", styles); z.writestr("word/settings.xml", settings)
