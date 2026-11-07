#!/usr/bin/env python3
"""Append a self-contained wrapping shape to a long input fixture for profiling."""

from __future__ import annotations

import argparse
from pathlib import Path
from xml.dom import minidom
from xml.parsers import expat
from zipfile import ZipFile

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
WP = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
XMLNS = "http://www.w3.org/2000/xmlns/"


def write_fixture(base: Path, shape: Path, output: Path) -> None:
    with ZipFile(base) as archive:
        original_xml = archive.read("word/document.xml")
        document = minidom.parseString(original_xml)
    with ZipFile(shape) as archive:
        donor = minidom.parseString(archive.read("word/document.xml"))
    donor_body = donor.getElementsByTagNameNS(W, "body")[0]
    paragraph = next(
        child for child in donor_body.childNodes
        if child.nodeType == child.ELEMENT_NODE and child.namespaceURI == W
        and child.localName == "p"
        and any(anchor.getElementsByTagNameNS(WP, "wrapSquare")
                for anchor in child.getElementsByTagNameNS(WP, "anchor"))
    )
    # The copied shape must not depend on media, hyperlinks, or other parts
    # absent from the original document's relationship set.
    for element in [paragraph, *paragraph.getElementsByTagName("*")]:
        if any(attribute.namespaceURI == R for attribute in element.attributes.values()):
            raise ValueError("wrapping paragraph must not reference relationships")
    copied = document.importNode(paragraph, deep=True)
    for attribute in donor.documentElement.attributes.values():
        if attribute.namespaceURI == XMLNS:
            copied.setAttributeNS(XMLNS, attribute.name, attribute.value)
    next_id = max((int(node.getAttribute("id"))
                   for node in document.getElementsByTagNameNS(WP, "docPr")), default=0) + 1
    for node in copied.getElementsByTagNameNS(WP, "docPr"):
        node.setAttribute("id", str(next_id))
        next_id += 1
    # Insert into the original UTF-8 bytes: reserializing the old document
    # would normalize whitespace in attributes such as embedded base64 data.
    original_xml.decode("utf-8")
    parser = expat.ParserCreate(namespace_separator="}")
    depth = 0
    offset = None

    def start(name, attributes):
        nonlocal depth, offset
        depth += 1
        if depth == 3 and name == W + "}sectPr":
            offset = parser.CurrentByteIndex

    def end(name):
        nonlocal depth, offset
        if depth == 2 and name == W + "}body" and offset is None:
            offset = parser.CurrentByteIndex
        depth -= 1

    parser.StartElementHandler = start
    parser.EndElementHandler = end
    parser.Parse(original_xml, True)
    if offset is None:
        raise ValueError("input has no document body")
    modified_xml = original_xml[:offset] + copied.toxml().encode("utf-8") + original_xml[offset:]
    output.parent.mkdir(parents=True, exist_ok=True)
    with ZipFile(base) as source, ZipFile(output, "w") as target:
        for entry in source.infolist():
            data = modified_xml if entry.filename == "word/document.xml" else source.read(entry)
            target.writestr(entry, data)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base", type=Path)
    parser.add_argument("shape", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    write_fixture(args.base, args.shape, args.output)
