#!/usr/bin/env python3
"""Word probe: the height of a line holding an inline picture or text box.

usage: make-object-lines-probe.py <table-object-edge-cases.docx> <out-dir>
Starts from public/fixtures/table-object-edge-cases.docx (Arial 10.5pt,
compatibility mode 14) and replaces its body. Each case is a one-line text
paragraph followed by a paragraph holding a 20pt or 60pt inline picture,
alone or after `x `, at single, 1.15 and 1.5 line spacing with no space
after; then inline VML text boxes 58pt tall with the default stroke and with
none, at the document's 1.15 lines and 10pt after.
"""
import os, re, sys, zipfile
src, out = sys.argv[1], sys.argv[2]
z = zipfile.ZipFile(src)
doc = z.read("word/document.xml").decode()
picture = re.search(r"<w:r><w:drawing>.*?</w:drawing></w:r>", doc, re.S).group(0)
def pic(points):
    emu = int(points * 12700)
    return re.sub(r'cx="\d+" cy="\d+"', f'cx="{emu}" cy="{emu}"', picture)
def text_box(extra):
    return ('<w:r><w:pict><v:shape type="#_x0000_t202" style="width:250pt;height:58pt" '
            f'strokecolor="#4472C4" fillcolor="#EAF2F8" {extra}><v:textbox inset="8pt,6pt,8pt,6pt">'
            '<w:txbxContent><w:p><w:r><w:t>Box</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r>')
def t(text):
    return f'<w:r><w:t xml:space="preserve">{text}</w:t></w:r>'
def p(runs, ppr=""):
    return f"<w:p>{ppr}{runs}</w:p>"
body, n = "", 0
for line in (240, 276, 360):
    ppr = f'<w:pPr><w:spacing w:after="0" w:line="{line}" w:lineRule="auto"/></w:pPr>'
    for points in (20, 60):
        for before in ("", "x "):
            n += 1
            body += p(t(f"B{n}"), ppr) + p((t(before) if before else "") + pic(points), ppr)
for extra in ("", 'stroked="f"'):
    n += 1
    body += p(t(f"B{n}")) + p(text_box(extra))
body += p(t("End"))
doc = re.sub(r"<w:body>.*?(<w:sectPr)", lambda m: "<w:body>" + body + m.group(1), doc, flags=re.S)
with zipfile.ZipFile(os.path.join(out, "word-mac-probe-object-lines.docx"), "w", zipfile.ZIP_DEFLATED) as o:
    for item in z.infolist():
        o.writestr(item.filename, doc if item.filename == "word/document.xml" else z.read(item.filename))
