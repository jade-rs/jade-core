#!/usr/bin/env python3
"""Writes Word probe documents for Mac punctuation squeeze (oikomi) rules.

usage: make-squeeze-probe.py <out-dir>

Every probe paragraph is justified MS Mincho text that wraps once. The right
indent sets the available width so that admitting the k-th character needs a
reduction of exactly D points; the Word PDF shows whether it was admitted.
manifest.json lists every paragraph in document order with its parameters.
"""
import json
import os
import sys
import zipfile

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
CONTENT_WIDTH_TW = 11906 - 2 * 1701


def run(text, size_half_points, kern):
    kern_xml = '<w:kern w:val="2"/>' if kern else ""
    return (
        f'<w:r><w:rPr><w:rFonts w:ascii="ＭＳ 明朝" w:eastAsia="ＭＳ 明朝" w:hAnsi="ＭＳ 明朝"/>'
        f'{kern_xml}<w:sz w:val="{size_half_points}"/><w:szCs w:val="{size_half_points}"/></w:rPr>'
        f'<w:t xml:space="preserve">{text}</w:t></w:r>'
    )


def paragraph(text, size_half_points, kern, right_tw, jc="both"):
    return (
        f'<w:p><w:pPr><w:spacing w:before="0" w:after="120"/>'
        f'<w:ind w:left="0" w:right="{right_tw}"/><w:jc w:val="{jc}"/></w:pPr>'
        f"{run(text, size_half_points, kern)}</w:p>"
    )


def package(path, body, compress):
    csc = "compressPunctuation" if compress else "doNotCompress"
    document = (
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<w:document xmlns:w="{W}"><w:body>{body}'
        f'<w:sectPr><w:pgSz w:w="11906" w:h="16838"/>'
        f'<w:pgMar w:top="1134" w:right="1701" w:bottom="1134" w:left="1701" w:header="851" w:footer="992" w:gutter="0"/>'
        f'<w:docGrid w:linePitch="360"/></w:sectPr></w:body></w:document>'
    )
    settings = (
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<w:settings xmlns:w="{W}"><w:defaultTabStop w:val="840"/>'
        f'<w:characterSpacingControl w:val="{csc}"/>'
        f'<w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat>'
        f"</w:settings>"
    )
    styles = (
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        f'<w:styles xmlns:w="{W}"><w:docDefaults><w:rPrDefault><w:rPr>'
        f'<w:rFonts w:ascii="ＭＳ 明朝" w:eastAsia="ＭＳ 明朝" w:hAnsi="ＭＳ 明朝" w:cs="Times New Roman"/>'
        f'<w:sz w:val="21"/><w:lang w:val="en-US" w:eastAsia="ja-JP"/></w:rPr></w:rPrDefault>'
        f'<w:pPrDefault><w:pPr><w:widowControl w:val="0"/></w:pPr></w:pPrDefault></w:docDefaults>'
        f'<w:style w:type="paragraph" w:default="1" w:styleId="a"><w:name w:val="Normal"/></w:style></w:styles>'
    )
    content_types = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
        '<Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/>'
        '<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>'
        "</Types>"
    )
    rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>'
        "</Relationships>"
    )
    doc_rels = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
        '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/>'
        '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>'
        "</Relationships>"
    )
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", content_types)
        z.writestr("_rels/.rels", rels)
        z.writestr("word/document.xml", document)
        z.writestr("word/_rels/document.xml.rels", doc_rels)
        z.writestr("word/settings.xml", settings)
        z.writestr("word/styles.xml", styles)


PAIRS = ["。）", "）。", "）、", "、「", "」（", "（「", "。「", "）」"]

# The k-th character (1-based) is the one admitted or pushed out. Blank
# sides in the first k-1 characters: three commas (3 x half em).
SQUEEZE_TEXTS = {
    # ordinary kana at position k
    "kana": "あいう、えおかき、くけこさ、しすせそたちつてとなにぬねのはひふへほまみむめも",
    # line-start-prohibited comma at position k
    "comma": "あいう、えおかき、くけこさ、しすせそ、たちつてとなにぬねのはひふへほまみむめも",
    # no blanks at all before k
    "plain": "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめも",
}
K = 19  # index (1-based) of the probed character in every text


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    manifest = {}
    for compress in (True, False):
        body = []
        entries = []
        for kern in (True, False):
            for pair in PAIRS:
                text = f"あい{pair}うえ"
                body.append(paragraph(text, 21, kern, 0, jc="left"))
                entries.append({"kind": "pair", "pair": pair, "kern": kern, "text": text})
        name = f"pairs-{'compress' if compress else 'nocompress'}.docx"
        package(os.path.join(out, name), "".join(body), compress)
        manifest[name] = entries
    for size_hp in (21, 24):
        em_tw = size_hp * 10  # half points -> twips (1pt = 20tw, size/2 pt)
        for kind, text in SQUEEZE_TEXTS.items():
            body = []
            entries = []
            # D from -2pt to 17pt in 0.25pt (5tw) steps
            for d_tw in range(-40, 345, 5):
                available_tw = K * em_tw - d_tw
                right_tw = CONTENT_WIDTH_TW - available_tw
                body.append(paragraph(text, size_hp, True, right_tw))
                entries.append({"kind": kind, "size_pt": size_hp / 2, "d_pt": d_tw / 20, "k": K, "text": text})
            name = f"squeeze-{kind}-{size_hp}.docx"
            package(os.path.join(out, name), "".join(body), True)
            manifest[name] = entries
    with open(os.path.join(out, "manifest.json"), "w") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=1)


main()
