"""Generate a one-variable-at-a-time probe for Word's vertical section origins."""

from copy import deepcopy
from pathlib import Path
from xml.etree import ElementTree as ET
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).parent
NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
ET.register_namespace("w", NS)


def w(name):
    return f"{{{NS}}}{name}"


with ZipFile(ROOT / "east-asian-compress.docx") as source:
    parts = {name: source.read(name) for name in source.namelist()}

document = ET.fromstring(parts["word/document.xml"])
body = document.find(w("body"))
paragraph, break_paragraph = deepcopy(body[0]), deepcopy(body[1])
body.clear()

variants = (
    "minimal",
    "complete margins",
    "asymmetric margins",
    "line grid",
    "18 pt",
)
for direction in ("tbRl", "tbRlV"):
    for variant in variants:
        content = deepcopy(paragraph)
        if variant == "18 pt":
            for size in content.iter(w("sz")):
                size.set(w("val"), "36")
        body.append(content)

        # A second column exposes the flow direction independently of the
        # first column's position. Keep its font and size equal to the first.
        second_column = deepcopy(content)
        for run in list(second_column.findall(w("r")))[1:]:
            second_column.remove(run)
        second_column.find(f"{w('r')}/{w('t')}").text = "次列"
        body.append(second_column)

        section_break = deepcopy(break_paragraph)
        section = section_break.find(f"{w('pPr')}/{w('sectPr')}")
        section.find(w("textDirection")).set(w("val"), direction)
        margins = section.find(w("pgMar"))
        if variant != "minimal":
            for side, value in (("header", "720"), ("footer", "720"), ("gutter", "0")):
                margins.set(w(side), value)
        if variant == "asymmetric margins":
            margins.set(w("left"), "1080")
            margins.set(w("right"), "2160")
        if variant == "line grid":
            ET.SubElement(section, w("docGrid"), {w("type"): "lines", w("linePitch"): "360"})
        if direction == "tbRlV" and variant == variants[-1]:
            body.append(section)
        else:
            body.append(section_break)

parts["word/document.xml"] = ET.tostring(document, encoding="utf-8", xml_declaration=True)
with ZipFile(ROOT / "vertical-section-origins.docx", "w") as output:
    for name, data in parts.items():
        entry = ZipInfo(name)
        entry.compress_type = ZIP_DEFLATED
        output.writestr(entry, data)
