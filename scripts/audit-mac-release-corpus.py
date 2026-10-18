#!/usr/bin/env python3
"""Audit one-character DOCX edits across the repository fixture corpus.

Run from the repository root: python3 scripts/audit-mac-release-corpus.py
The Rust example performs the actual editor command and writes fresh outputs.
This check detects package-part loss, opaque payload changes, and unexpected
changes to visible, deleted, or field-instruction text. It cannot establish
Word rendering fidelity or equivalence of all non-text XML.
"""

from collections import Counter
from pathlib import Path
import posixpath
import subprocess
import tempfile
from urllib.parse import unquote, urlsplit
import xml.etree.ElementTree as ET
from zipfile import ZipFile


ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "public/fixtures"
MINIMUM_FIXTURE_COUNT = 114
WORD = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
XML = "{http://www.w3.org/XML/1998/namespace}"
GFXDATA = "{urn:schemas-microsoft-com:office:office}gfxdata"
BOOLEAN_TAGS = {WORD + "strike", WORD + "outline"}
BOOLEAN_VALUES = {"1": "true", "on": "true", "yes": "true", "0": "false", "off": "false", "no": "false"}
# Property containers whose children the export writes in ECMA-376 sequence
# order (jade-core/crates/jade-docx-reader/src/docx_xml/schema.rs). An export may move a
# source's out-of-order property into place; it must not add, drop or change one.
PROPERTY_CONTAINERS = {
    WORD + name
    for name in (
        "pPr", "sectPr", "tblPr", "trPr", "tcPr", "tblPrEx", "pBdr", "pgBorders",
        "tblBorders", "tcBorders", "tblCellMar", "tcMar", "numPr", "footnotePr",
        "endnotePr",
    )
}
# Revision elements whose `w:id` Word requires to be unique. A source that
# repeats one is renumbered on export, so their ids are compared separately.
REVISION_ELEMENTS = {
    WORD + name
    for name in (
        "ins", "del", "moveFrom", "moveTo", "rPrChange", "pPrChange", "sectPrChange",
        "tblPrChange", "trPrChange", "tcPrChange", "tblGridChange", "tblPrExChange",
        "numberingChange", "cellIns", "cellDel", "cellMerge",
    )
}
RELATIONSHIP = "{http://schemas.openxmlformats.org/package/2006/relationships}Relationship"
CONTENT_TYPE = "{http://schemas.openxmlformats.org/package/2006/content-types}"


def text_sequence(xml: bytes, tag: str) -> list[str]:
    root = ET.fromstring(xml)
    return [
        element.text or "" if element.get(XML + "space") == "preserve"
        else (element.text or "").strip(" \t\r\n")
        for element in root.iter(WORD + tag)
    ]


def marker_sequence(xml: bytes, tag: str) -> list[tuple[tuple[str, str], ...]]:
    return [
        tuple(sorted(element.attrib.items()))
        for element in ET.fromstring(xml).iter(WORD + tag)
    ]


def structural_sequence(
    xml: bytes, include_run_properties: bool = True
) -> list[tuple[str, tuple[tuple[str, str], ...]]]:
    """Non-text elements with their attributes, in document order.

    `include_run_properties=False` is the order check: it skips run properties
    and compares the children of a property container (`PROPERTY_CONTAINERS`)
    as a set, since the export puts them in schema order. Revision ids are left
    out; `revision_id_errors` checks them.
    """

    def visit(element: ET.Element) -> list[tuple[str, tuple[tuple[str, str], ...]]]:
        if element.tag == WORD + "rPr" and not include_run_properties:
            return []
        sequence: list[tuple[str, tuple[tuple[str, str], ...]]] = []
        if element.tag in (WORD + "r", WORD + "t"):
            pass
        elif (
            element.tag == WORD + "rPr"
            and not element.attrib
            and not len(element)
            and not (element.text or "").strip()
        ):
            pass
        else:
            attributes = []
            for name, value in element.attrib.items():
                if name == GFXDATA:
                    value = "".join(value.split())
                elif element.tag in BOOLEAN_TAGS and name == WORD + "val":
                    value = BOOLEAN_VALUES.get(value, value)
                elif element.tag in REVISION_ELEMENTS and name == WORD + "id":
                    continue
                attributes.append((name, value))
            if element.tag in BOOLEAN_TAGS and WORD + "val" not in element.attrib:
                attributes.append((WORD + "val", "true"))
            sequence.append((element.tag, tuple(sorted(attributes))))
        children = [visit(child) for child in element]
        if element.tag in PROPERTY_CONTAINERS and not include_run_properties:
            children.sort()
        for child in children:
            sequence.extend(child)
        return sequence

    return visit(ET.fromstring(xml))


# What a run holds besides text: typed beside one of these, text takes a run of
# its own rather than sharing the object's, as Word does.
OBJECT_TAGS = {
    WORD + "drawing", WORD + "pict", WORD + "object", WORD + "footnoteReference",
    WORD + "endnoteReference", WORD + "footnoteRef", WORD + "endnoteRef",
    "{http://schemas.openxmlformats.org/markup-compatibility/2006}AlternateContent",
    "{http://schemas.openxmlformats.org/officeDocument/2006/math}oMath",
}


def without_run_typed_beside_object(xml: bytes, typed: str) -> bytes:
    """`xml` without the one run that holds exactly `typed` right after a run
    holding an object, the run the edit may add; any other new run is still
    a structural change."""
    root = ET.fromstring(xml)
    for parent in root.iter():
        children = list(parent)
        for previous, run in zip(children, children[1:]):
            if run.tag != WORD + "r" or previous.tag != WORD + "r":
                continue
            content = [child for child in run if child.tag != WORD + "rPr"]
            holds_typed = (
                len(content) == 1 and content[0].tag == WORD + "t" and content[0].text == typed
            )
            if holds_typed and any(element.tag in OBJECT_TAGS for element in previous.iter()):
                parent.remove(run)
                return ET.tostring(root)
    return xml


def revision_id_errors(before: bytes, after: bytes) -> list[str]:
    """Revision ids must survive an edit, except that a source which repeats
    one gets the repeats renumbered, and every id in the export is unique."""

    def ids(xml: bytes) -> list[tuple[str, str]]:
        return [
            (element.tag, element.attrib[WORD + "id"])
            for element in ET.fromstring(xml).iter()
            if element.tag in REVISION_ELEMENTS and WORD + "id" in element.attrib
        ]

    failures = []
    source = ids(before)
    repeats = Counter(value for _, value in source)
    unique = Counter(entry for entry in source if repeats[entry[1]] == 1)
    exported = ids(after)
    if unique - Counter(exported):
        failures.append("changed revision IDs")
    if any(count > 1 for count in Counter(value for _, value in exported).values()):
        failures.append("repeated revision IDs")
    return failures


def attribute_sequence(xml: bytes, attribute: str) -> list[str]:
    return [
        element.attrib[WORD + attribute]
        for element in ET.fromstring(xml).iter()
        if WORD + attribute in element.attrib
    ]


def missing_relationship_targets(package: ZipFile) -> list[str]:
    parts = set(package.namelist())
    missing = []
    for rels_part in sorted(part for part in parts if part.endswith(".rels")):
        if rels_part == "_rels/.rels":
            source_dir = ""
            source_part = ""
        else:
            parent, marker, filename = rels_part.rpartition("/_rels/")
            if not marker:
                continue
            source_dir = parent
            source_part = posixpath.join(parent, filename.removesuffix(".rels"))
        for relationship in ET.fromstring(package.read(rels_part)).iter(RELATIONSHIP):
            if relationship.get("TargetMode") == "External":
                continue
            target = relationship.get("Target", "")
            uri = urlsplit(target)
            if uri.scheme or uri.netloc:
                missing.append(f"{rels_part}: internal target has a URI scheme: {target}")
                continue
            if not uri.path:
                resolved = source_part
            elif uri.path.startswith("/"):
                resolved = posixpath.normpath(unquote(uri.path).lstrip("/"))
            else:
                resolved = posixpath.normpath(posixpath.join(source_dir, unquote(uri.path)))
            if resolved == ".." or resolved.startswith("../") or resolved not in parts:
                missing.append(f"{rels_part}: missing internal target {target} ({resolved})")
    return missing


def parts_without_content_type(package: ZipFile) -> set[str]:
    parts = {name for name in package.namelist() if not name.endswith("/")}
    if "[Content_Types].xml" not in parts:
        return parts
    root = ET.fromstring(package.read("[Content_Types].xml"))
    defaults = {
        element.get("Extension", "").lower()
        for element in root.iter(CONTENT_TYPE + "Default")
    }
    overrides = {
        element.get("PartName", "").lstrip("/")
        for element in root.iter(CONTENT_TYPE + "Override")
    }
    return {
        name
        for name in parts - {"[Content_Types].xml"}
        if name not in overrides and name.rpartition(".")[-1].lower() not in defaults
    }


def package_structure_errors(package: ZipFile, label: str) -> list[str]:
    names = package.namelist()
    errors = []
    if duplicates := sorted(name for name, count in Counter(names).items() if count > 1):
        errors.append(f"{label} duplicate parts: {duplicates}")
    for name in sorted(set(names)):
        if name.endswith((".xml", ".rels")) or name == "[Content_Types].xml":
            try:
                ET.fromstring(package.read(name))
            except ET.ParseError as error:
                errors.append(f"{label} invalid XML in {name}: {error}")
    return errors


def audit(source: Path, exported: Path) -> list[str]:
    failures = []
    with ZipFile(source) as original, ZipFile(exported) as edited:
        failures.extend(package_structure_errors(original, "source"))
        failures.extend(package_structure_errors(edited, "edited"))
        if failures:
            return failures
        failures.extend(missing_relationship_targets(edited))
        if untyped_source := parts_without_content_type(original):
            failures.append(f"source parts without content types: {sorted(untyped_source)}")
        if untyped_edited := parts_without_content_type(edited):
            failures.append(f"edited parts without content types: {sorted(untyped_edited)}")
        original_parts = set(original.namelist())
        edited_parts = set(edited.namelist())
        if missing := original_parts - edited_parts:
            failures.append(f"lost parts: {sorted(missing)}")
        for part in sorted(original_parts & edited_parts):
            if not (part.endswith(".xml") or part.endswith(".rels") or part == "[Content_Types].xml"):
                if original.read(part) != edited.read(part):
                    failures.append(f"changed opaque part: {part}")
        if "word/document.xml" in original_parts & edited_parts:
            before = original.read("word/document.xml")
            after = edited.read("word/document.xml")
            for tag in ("delText", "instrText"):
                if text_sequence(before, tag) != text_sequence(after, tag):
                    failures.append(f"changed {tag}")
            if "".join(text_sequence(after, "t")) != "Z" + "".join(text_sequence(before, "t")):
                failures.append("ordinary text differs from the one-character edit")
            for tag in ("cr", "br", "tab", "ptab", "softHyphen", "noBreakHyphen"):
                if marker_sequence(before, tag) != marker_sequence(after, tag):
                    failures.append(f"changed {tag} marker sequence or attributes")
            structure = without_run_typed_beside_object(after, "Z")
            if Counter(structural_sequence(before)) != Counter(structural_sequence(structure)):
                failures.append("changed non-text document structure or attributes")
            if structural_sequence(before, False) != structural_sequence(structure, False):
                failures.append("changed non-text document order outside run properties")
            failures.extend(revision_id_errors(before, after))
            for attribute in ("rsidR", "rsidRPr", "rsidDel"):
                if attribute_sequence(before, attribute) != attribute_sequence(after, attribute):
                    failures.append(f"changed {attribute} run attributes")
    return failures


def main() -> None:
    fixtures = sorted(FIXTURES.glob("*.docx"))
    if len(fixtures) < MINIMUM_FIXTURE_COUNT:
        raise SystemExit(
            f"fixture corpus shrank: found {len(fixtures)} DOCX files, expected at least {MINIMUM_FIXTURE_COUNT}"
        )
    with tempfile.TemporaryDirectory(prefix="jade-mac-release-audit-") as directory:
        output = Path(directory)
        subprocess.run(
            ["cargo", "run", "-p", "jade-docx-editor", "--example", "mac_release_roundtrip", "--", str(output)],
            cwd=ROOT,
            check=True,
        )
        generated = {path.name for path in output.glob("*.docx")}
        expected = {path.name for path in fixtures}
        failures = [f"missing edited fixture: {name}" for name in sorted(expected - generated)]
        failures += [f"unexpected edited fixture: {name}" for name in sorted(generated - expected)]
        for fixture in fixtures:
            if fixture.name in generated:
                failures.extend(f"{fixture.name}: {failure}" for failure in audit(fixture, output / fixture.name))
    if failures:
        raise SystemExit("\n".join(failures))
    print(f"PASS: {len(fixtures)} edited DOCX fixtures preserved package parts, content type coverage, internal relationship targets, opaque payloads, deleted text, field instructions, break/tab marker sequences and attributes, non-text structure and attributes, order outside run and property containers, unique revision IDs, run revision IDs, and expected ordinary text")


if __name__ == "__main__":
    main()
