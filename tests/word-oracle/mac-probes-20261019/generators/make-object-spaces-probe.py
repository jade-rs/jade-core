#!/usr/bin/env python3
"""Word probe: ASCII spaces next to an inline picture.

usage: make-object-spaces-probe.py <image-inline.docx> <out-dir>
Starts from public/fixtures/image-inline.docx (Yu Mincho 10.5pt, a Japanese
template whose settings turn on w:balanceSingleByteDoubleByteWidth) and
replaces its body with paragraphs that put a 20pt inline picture next to
spaces: before and after it, in lines whose other spaces are away from it,
and with a floating picture instead.
"""
import os, re, sys, zipfile
src, out = sys.argv[1], sys.argv[2]
z = zipfile.ZipFile(src)
doc = z.read("word/document.xml").decode()
pic = re.search(r"<w:r><w:rPr><w:noProof/></w:rPr><w:drawing>.*?</w:drawing></w:r>", doc, re.S).group(0)
pic = pic.replace("1778000", "254000")  # 140pt -> 20pt
floating = re.sub(
    r"<wp:inline[^>]*>",
    '<wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" relativeHeight="1" '
    'behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/>'
    '<wp:positionH relativeFrom="margin"><wp:posOffset>4000000</wp:posOffset></wp:positionH>'
    '<wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV>',
    pic,
).replace("</wp:inline>", "</wp:anchor>")
floating = re.sub(r"(<wp:effectExtent[^>]*/>)", r"\1<wp:wrapNone/>", floating)
def r(text, rpr=""):
    return f'<w:r>{rpr}<w:t xml:space="preserve">{text}</w:t></w:r>'
words = "Aa bb cc dd ee gg hh ii jj kk ll mm nn oo pp qq rr ss tt uu vv ww xx yy zz"
paragraphs = [
    r("Hello") + r(" ") + pic + r("World"),
    r("Hello ") + pic + r(" World"),
    r("Hello World ") + pic,
    pic + r(" " + words),
    r(words + " ") + pic + r(" end"),
    r("Hello World ") + floating + r("x y"),
]
body = "".join(f"<w:p>{p}</w:p>" for p in paragraphs)
doc = re.sub(r"<w:body>.*?(<w:sectPr)", lambda m: "<w:body>" + body + m.group(1), doc, flags=re.S)
with zipfile.ZipFile(os.path.join(out, "word-mac-probe-object-spaces.docx"), "w", zipfile.ZIP_DEFLATED) as o:
    for item in z.infolist():
        o.writestr(item.filename, doc if item.filename == "word/document.xml" else z.read(item.filename))
