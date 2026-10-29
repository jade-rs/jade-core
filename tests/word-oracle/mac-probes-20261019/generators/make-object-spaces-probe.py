#!/usr/bin/env python3
"""Generate Word probes for ASCII spaces next to an inline picture.

usage: make-object-spaces-probe.py <image-inline.docx> <out-dir> [--space-runs]
The default preserves the six previously observed paragraphs. --space-runs
adds consecutive-space cases and emits matched documents with and without
balanceSingleByteDoubleByteWidth, plus a paragraph manifest. These are inputs
for fresh Word measurements; generating them does not establish layout parity.
"""
import argparse
from html import escape
import json
from pathlib import Path
import re
from xml.etree import ElementTree as ET
import zipfile

W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"


def r(text):
    return f'<w:r><w:t xml:space="preserve">{escape(text)}</w:t></w:r>'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--space-runs", action="store_true")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(args.source) as archive:
        doc = archive.read("word/document.xml").decode()
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
        words = "Aa bb cc dd ee gg hh ii jj kk ll mm nn oo pp qq rr ss tt uu vv ww xx yy zz"
        cases = [
            ("split-single-before", r("Hello") + r(" ") + pic + r("World")),
            ("single-both", r("Hello ") + pic + r(" World")),
            ("paragraph-end", r("Hello World ") + pic),
            ("paragraph-start", pic + r(" " + words)),
            ("long-line", r(words + " ") + pic + r(" end")),
            ("floating-single", r("Hello World ") + floating + r("x y")),
        ]
        if args.space_runs:
            cases += [
                ("two-before", r("Hello  ") + pic + r("World")),
                ("three-before", r("Hello   ") + pic + r("World")),
                ("two-after", r("Hello") + pic + r("  World")),
                ("three-after", r("Hello") + pic + r("   World")),
                ("split-three-before", r("Hello ") + r(" ") + r(" ") + pic + r("World")),
                ("split-three-after", r("Hello") + pic + r(" ") + r(" ") + r(" World")),
                ("floating-three", r("Hello   ") + floating + r("World")),
                ("text-three", r("Hello   World")),
            ]
            drawing_id = 0

            def unique_id(match):
                nonlocal drawing_id
                drawing_id += 1
                return match.group(1) + str(drawing_id) + match.group(2)

            cases = [(name, re.sub(r'(<wp:docPr\b[^>]*\bid=")[^"]+(")', unique_id, xml))
                     for name, xml in cases]
        body = "".join(f"<w:p>{xml}</w:p>" for _, xml in cases)
        doc = re.sub(r"<w:body>.*?(<w:sectPr)", lambda m: "<w:body>" + body + m.group(1), doc, flags=re.S)
        settings = archive.read("word/settings.xml").decode()
        variants = [("word-mac-probe-object-spaces.docx", settings)]
        if args.space_runs:
            if ET.fromstring(settings).find(".//" + W + "balanceSingleByteDoubleByteWidth") is None:
                raise ValueError("Expected a template with balanced single-byte spaces enabled")
            unbalanced = re.sub(r"<w:balanceSingleByteDoubleByteWidth\b[^>]*/>", "", settings)
            variants = [(f"word-mac-probe-object-space-runs-{variant}.docx", value)
                        for variant, value in [("balanced", settings), ("unbalanced", unbalanced)]]
            manifest = []
            xml_paragraphs = ET.fromstring(doc).find(W + "body").findall(W + "p")
            for (name, _), paragraph in zip(cases, xml_paragraphs):
                text = "".join(child.text or "" if child.tag == W + "t" else "\ufffc"
                               for run in paragraph for child in run if child.tag in {W + "t", W + "drawing"})
                manifest.append({"id": name, "text": text})
            (args.output / "word-mac-probe-object-space-runs.cases.json").write_text(
                json.dumps({"paragraphs": manifest}, indent=2, ensure_ascii=False) + "\n")
        for filename, variant_settings in variants:
            with zipfile.ZipFile(args.output / filename, "w", zipfile.ZIP_DEFLATED) as output:
                for item in archive.infolist():
                    value = {"word/document.xml": doc, "word/settings.xml": variant_settings}.get(item.filename)
                    output.writestr(item.filename, value if value is not None else archive.read(item.filename))


if __name__ == "__main__":
    main()
