#!/usr/bin/env python3
"""Word probe: a hanging-indent tab in a full justified line.

usage: make-tab-shrink-probe.py <out-dir>
Justified paragraphs `1．<tab>` + East Asian text under a hanging indent
(trust-beneficiary-sale-keep-next's margins, 10.5pt MS Mincho, doNotCompress).
The left indent steps by 20 twips so the text after the tab overflows the first
line by a little more each time; the first line ends in a kanji (tabshrink-kanji)
or a comma (tabshrink-comma). The tab's width and where the line breaks show how
Word fits an overflowing line that holds a tab.
"""
import os, sys, zipfile
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
FILL = "本合意書の当事者は本合意書締結日において自ら及び自らの役員取締役執行役員業務を執行する社員その他これらに準ずる者"
def doc(ending):
    body = ""
    for i, left in enumerate(range(560, 900, 20)):
        first = FILL[:40] + ("、" if ending == "comma" else "員")
        text = first + "以下同じ。" + str(i)
        body += (f'<w:p><w:pPr><w:ind w:left="{left}" w:hanging="{left - 1}"/><w:jc w:val="both"/></w:pPr>'
                 f'<w:r><w:t>1．</w:t></w:r><w:r><w:tab/></w:r><w:r><w:t>{text}</w:t></w:r></w:p>')
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="{W}"><w:body>{body}'
            '<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1418" w:right="1418" w:bottom="1418" '
            'w:left="1365" w:header="851" w:footer="851" w:gutter="0"/><w:docGrid w:linePitch="326"/></w:sectPr></w:body></w:document>')
styles = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="{W}"><w:docDefaults>'
          '<w:rPrDefault><w:rPr><w:rFonts w:ascii="ＭＳ 明朝" w:eastAsia="ＭＳ 明朝" w:hAnsi="ＭＳ 明朝" w:cs="Times New Roman"/>'
          '<w:kern w:val="2"/><w:sz w:val="21"/><w:szCs w:val="24"/><w:lang w:val="en-US" w:eastAsia="ja-JP" w:bidi="ar-SA"/>'
          '</w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults>'
          '<w:style w:type="paragraph" w:default="1" w:styleId="a"><w:name w:val="Normal"/><w:pPr><w:widowControl w:val="0"/>'
          '</w:pPr></w:style></w:styles>')
settings = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:w="{W}"><w:defaultTabStop w:val="840"/>'
            '<w:characterSpacingControl w:val="doNotCompress"/><w:compat><w:compatSetting w:name="compatibilityMode" '
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
for ending in ("kanji", "comma"):
    with zipfile.ZipFile(os.path.join(out, f"tabshrink-{ending}.docx"), "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", ct); z.writestr("_rels/.rels", rels)
        z.writestr("word/_rels/document.xml.rels", drels); z.writestr("word/document.xml", doc(ending))
        z.writestr("word/styles.xml", styles); z.writestr("word/settings.xml", settings)
