#!/usr/bin/env python3
"""Extract relative script baselines from the captured Word PDF (PyMuPDF)."""

import csv
import re
from pathlib import Path

import pymupdf

root = Path(__file__).resolve().parent
label = re.compile(r"(ordinary|ruby)-(superscript|subscript)-(\d+):")
rows = []

with pymupdf.open(root / "arial-script-sweep-word.pdf") as document:
    for page in document:
        origins = {}
        current = None
        for block in page.get_text("dict")["blocks"]:
            for line in block.get("lines", []):
                for span in line["spans"]:
                    text = span["text"].strip()
                    match = label.fullmatch(text)
                    if match:
                        assert current is None, current
                        current = match.groups()
                        origins = {}
                    elif text in ("BASE", "ABC"):
                        assert current is not None and text not in origins, text
                        origins[text] = span["origin"][1]
                        if len(origins) == 2:
                            rows.append((*current, f'{origins["BASE"] - origins["ABC"]:.4f}'))
                            current = None
        assert current is None, current

assert len(rows) == 196
assert len({row[:3] for row in rows}) == 196
with (root / "word-baselines.csv").open("w", newline="") as output:
    writer = csv.writer(output, lineterminator="\n")
    writer.writerow(("context", "script", "size_half_points", "base_minus_script_pt"))
    writer.writerows(rows)
