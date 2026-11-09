#!/usr/bin/env python3
"""Reduces a scripts/word-pdf-glyph-reference.py reference to each row's page, baseline and text.

usage: compact_page_reference.py <reference.json> <out.json>
A 265-page document's glyph reference is 22MB; its pagination fits in a fraction of that.
"""
import json
import sys

reference = json.load(open(sys.argv[1], encoding="utf-8"))
reference["pages"] = [
    {
        "page": page["page"],
        "rows": [[round(row["baseline_pt"], 2), row["text"]] for row in page["rows"]],
    }
    for page in reference["pages"]
]
with open(sys.argv[2], "w", encoding="utf-8") as out:
    json.dump(reference, out, ensure_ascii=False, indent=0, separators=(",", ":"))
    out.write("\n")
