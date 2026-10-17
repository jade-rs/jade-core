#!/usr/bin/env python3
"""Word probe: paragraph indents (twips vs *Chars) over numbering-level indents,
and the font of a numbering label's space suffix.

usage: make-indent-probe.py <out-dir>
Every paragraph wraps to two lines so the first-line and continuation origins
are both visible. manifest.json lists the paragraphs in document order.
"""
import json
import os
import sys
import zipfile

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
TEXT = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをん"

CASES = [
    # (name, numbered, ind attributes, mark rPr)
    ("num-left-firstline", True, 'w:left="420" w:firstLine="4"', ""),
    ("num-chars-firstline", True, 'w:leftChars="200" w:left="420" w:firstLineChars="2" w:firstLine="4"', ""),
    ("num-chars-disagree-hanging", True, 'w:leftChars="202" w:left="565" w:hangingChars="67" w:hanging="141"', ""),
    ("num-twips-hanging", True, 'w:left="565" w:hanging="141"', ""),
    ("num-leftchars-disagree", True, 'w:leftChars="202" w:left="565"', ""),
    ("num-chars-only-hanging", True, 'w:leftChars="202" w:hangingChars="67"', ""),
    ("num-none", True, None, ""),
    ("plain-chars-disagree-hanging", False, 'w:leftChars="202" w:left="565" w:hangingChars="67" w:hanging="141"', ""),
    ("plain-leftchars-disagree", False, 'w:leftChars="202" w:left="565"', ""),
    ("num-mark-ascii-mincho", True, None, '<w:rFonts w:ascii="ＭＳ 明朝" w:hAnsi="ＭＳ 明朝"/>'),
    ("num-mark-plain", True, None, ""),
]


def paragraph(name, numbered, ind, mark):
    num = '<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>' if numbered else ""
    ind_xml = f"<w:ind {ind}/>" if ind else ""
    mark_xml = f"<w:rPr>{mark}</w:rPr>" if mark else ""
    return (
        f'<w:p><w:pPr>{num}<w:spacing w:before="0" w:after="120"/>{ind_xml}{mark_xml}</w:pPr>'
        f'<w:r><w:t>{TEXT}</w:t></w:r></w:p>'
    )


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    body = "".join(paragraph(*case) for case in CASES)
    document = (
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}'
        '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1134" w:right="1701" w:bottom="1134" '
        'w:left="1701" w:header="851" w:footer="992" w:gutter="0"/><w:docGrid w:type="lines" w:linePitch="360"/>'
        "</w:sectPr></w:body></w:document>"
    )
    numbering = (
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering xmlns:w="{W}">'
        '<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="hybridMultilevel"/>'
        '<w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimalEnclosedCircle"/><w:suff w:val="space"/>'
        '<w:lvlText w:val="%1"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="533" w:hanging="113"/></w:pPr>'
        '<w:rPr><w:rFonts w:hint="eastAsia"/></w:rPr></w:lvl></w:abstractNum>'
        '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>'
    )
    styles = (
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}"><w:docDefaults>'
        '<w:rPrDefault><w:rPr><w:rFonts w:ascii="Century" w:eastAsia="ＭＳ 明朝" w:hAnsi="Century" w:cs="Times New Roman"/>'
        '<w:kern w:val="2"/><w:sz w:val="21"/><w:szCs w:val="24"/><w:lang w:val="en-US" w:eastAsia="ja-JP" w:bidi="ar-SA"/>'
        "</w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults>"
        '<w:style w:type="paragraph" w:default="1" w:styleId="a"><w:name w:val="Normal"/><w:pPr><w:widowControl w:val="0"/>'
        '<w:jc w:val="both"/></w:pPr></w:style></w:styles>'
    )
    settings = (
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:w="{W}"><w:defaultTabStop w:val="840"/>'
        '<w:characterSpacingControl w:val="compressPunctuation"/><w:compat><w:compatSetting w:name="compatibilityMode" '
        'w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>'
    )
    ct = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
        '<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>'
        '<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>'
        '<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>'
        "</Types>"
    )
    rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>'
        "</Relationships>"
    )
    doc_rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/>'
        '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>'
        '<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>'
        "</Relationships>"
    )
    with zipfile.ZipFile(os.path.join(out, "indent-probe.docx"), "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", ct)
        z.writestr("_rels/.rels", rels)
        z.writestr("word/document.xml", document)
        z.writestr("word/_rels/document.xml.rels", doc_rels)
        z.writestr("word/settings.xml", settings)
        z.writestr("word/styles.xml", styles)
        z.writestr("word/numbering.xml", numbering)
    json.dump([c[0] for c in CASES], open(os.path.join(out, "manifest.json"), "w"), indent=1)


main()
