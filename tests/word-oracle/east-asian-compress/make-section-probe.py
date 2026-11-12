"""Generate a one-variable-at-a-time probe for Word's vertical section origins."""

from copy import deepcopy
from pathlib import Path
from xml.etree import ElementTree as ET
from zipfile import ZIP_DEFLATED, ZipFile, ZipInfo

ROOT = Path(__file__).parent
FIXTURES = ROOT.parent / "mac-elsa-synthetic"
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
    "plain CJK",
    "two digits",
    "four digits",
    "six digits",
)
for direction in ("tbRl", "tbRlV", "tbLrV"):
    for variant in variants:
        content = deepcopy(paragraph)
        if variant == "18 pt":
            for size in content.iter(w("sz")):
                size.set(w("val"), "36")
        runs = content.findall(w("r"))
        if variant == "plain CJK":
            content.remove(runs[1])
            content.remove(runs[3])
        elif variant == "two digits":
            content.remove(runs[3])
        elif variant in ("four digits", "six digits"):
            length = 4 if variant == "four digits" else 6
            digits = runs[3].find(w("t"))
            digits.text = digits.text[:length]
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
        if direction == "tbLrV" and variant == variants[-1]:
            body.append(section)
        else:
            body.append(section_break)

def write_docx(path):
    parts["word/document.xml"] = ET.tostring(document, encoding="utf-8", xml_declaration=True)
    with ZipFile(path, "w") as output:
        for name, data in parts.items():
            entry = ZipInfo(name)
            entry.compress_type = ZIP_DEFLATED
            output.writestr(entry, data)


write_docx(ROOT / "vertical-section-origins.docx")

origins = list(body)
body.clear()
for direction_index in range(2):
    for pitch in (12, 16, 18, 20, 24):
        for variant_index in (0, 5, 7, 8):
            start = (direction_index * len(variants) + variant_index) * 3
            content, second_column, section_break = deepcopy(origins[start:start + 3])
            body.extend((content, second_column))
            section = section_break.find(f"{w('pPr')}/{w('sectPr')}")
            ET.SubElement(section, w("docGrid"), {w("type"): "lines", w("linePitch"): str(pitch * 20)})
            if direction_index == 1 and pitch == 24 and variant_index == 8:
                body.append(section)
            else:
                body.append(section_break)

write_docx(FIXTURES / "vertical-grid-pitch-matrix-mac" / "source.docx")

grid_matrix = list(body)
spacing_cases = (
    ("snap off", None, False),
    ("exact", 480, False),
    ("atLeast", 480, False),
    ("auto", 480, False),
    ("empty before", None, False),
    ("wrap", None, False),
)
exact_cases = tuple(
    ("exact", pitch * 20, disable_grid)
    for disable_grid in (False, True)
    for pitch in (12, 24, 36)
)
for fixture, cases in (
    ("vertical-grid-spacing-probe-mac", spacing_cases),
    ("vertical-exact-column-probe-mac", exact_cases),
):
    body.clear()
    for direction_index in range(2):
        start = direction_index * 60
        for case_index, (rule, line, disable_grid) in enumerate(cases):
            content, second_column, section_break = deepcopy(grid_matrix[start:start + 3])
            props = content.find(w("pPr"))
            if props is None:
                props = ET.Element(w("pPr"))
                content.insert(0, props)
            if line is not None:
                ET.SubElement(props, w("spacing"), {w("line"): str(line), w("lineRule"): rule})
            elif rule == "snap off":
                ET.SubElement(props, w("snapToGrid"), {w("val"): "0"})
            elif rule == "empty before":
                body.append(ET.Element(w("p")))
            elif rule == "wrap":
                content.find(f"{w('r')}/{w('t')}").text *= 58
            body.extend((content, second_column))
            section = section_break.find(f"{w('pPr')}/{w('sectPr')}")
            if disable_grid:
                section.remove(section.find(w("docGrid")))
            if direction_index == 1 and case_index == len(cases) - 1:
                body.append(section)
            else:
                body.append(section_break)
    write_docx(FIXTURES / fixture / "source.docx")
