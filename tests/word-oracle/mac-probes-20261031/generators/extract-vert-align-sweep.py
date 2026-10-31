#!/usr/bin/env python3
"""Read Word's PDF of word-mac-probe-vert-align-sweep.docx (or -sweep-2).

usage: extract-vert-align-sweep.py <pdf> <docx> <out.json>
Word for Mac writes each run with its own text matrix in 1/300in device
units (0.24pt), so the run sizes and baselines are read straight from the
content streams; no glyph boxes are involved. Each paragraph has three runs
(`A`, the shifted run, `B`); the shifted run's alignment, faces, size and
text are read from the DOCX, in the same order.
"""
import hashlib, json, re, sys, zipfile, zlib
import xml.etree.ElementTree as ET
W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
pdf, docx, out = sys.argv[1:4]
shifted_runs = []
for paragraph in ET.fromstring(zipfile.ZipFile(docx).read("word/document.xml")).iter(W + "p"):
    run = paragraph.findall(W + "r")[1]
    fonts = run.find(f"{W}rPr/{W}rFonts")
    shifted_runs.append({
        "vertAlign": run.find(f"{W}rPr/{W}vertAlign").get(W + "val"),
        "face": fonts.get(W + "ascii"),
        "eastAsia": fonts.get(W + "eastAsia"),
        "text": run.find(W + "t").text,
        "sizeHalfPoints": int(run.find(f"{W}rPr/{W}sz").get(W + "val")),
    })
data = open(pdf, "rb").read()
runs = []
for match in re.finditer(rb"stream\r?\n", data):
    end = data.find(b"endstream", match.end())
    try:
        stream = zlib.decompress(data[match.end():end]).decode("latin1")
    except zlib.error:
        continue
    for t in re.finditer(r"([-\d.]+) 0 0 [-\d.]+ [-\d.]+ ([-\d.]+)\s+Tm\s*/\w+ 1 Tf \((.*?)\) Tj", stream):
        if t.group(3) != " ":  # paragraph marks
            runs.append((float(t.group(1)), float(t.group(2))))
assert len(runs) == 3 * len(shifted_runs), (len(runs), len(shifted_runs))
half_points = lambda du: round(du * 0.24 * 2)
rows = []
for n, shifted_run in enumerate(shifted_runs):
    (body, base), (script, shifted), (_, after) = runs[3 * n:3 * n + 3]
    assert base == after, n
    superscript = shifted_run["vertAlign"] == "superscript"
    shift = shifted - base if superscript else base - shifted
    rows.append({
        **shifted_run,
        "bodyDeviceUnits": body,
        "scriptDeviceUnits": script,
        "scriptHalfPoints": half_points(script),
        "shiftDeviceUnits": shift,
        "shiftHalfPoints": half_points(shift),
    })
sha = lambda path: hashlib.sha256(open(path, "rb").read()).hexdigest()
json.dump({"docxSha256": sha(docx), "pdfSha256": sha(pdf), "deviceUnitPt": 0.24, "rows": rows},
          open(out, "w"), ensure_ascii=False, indent=1)
