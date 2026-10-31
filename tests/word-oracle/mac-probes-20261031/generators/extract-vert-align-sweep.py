#!/usr/bin/env python3
"""Read Word's PDF of word-mac-probe-vert-align-sweep.docx.

usage: extract-vert-align-sweep.py <pdf> <docx> <out.json>
Word for Mac writes each run with its own text matrix in 1/300in device
units (0.24pt), so the run sizes and baselines are read straight from the
content streams; no glyph boxes are involved. Paragraphs come in the
generator's order: superscript then subscript, faces in FACES order, sizes
6 to 30pt by half points, three runs each (`A`, the shifted run, `B`).
"""
import hashlib, json, re, sys, zlib
FACES = ["游明朝", "Century", "ＭＳ 明朝", "Calibri"]
pdf, docx, out = sys.argv[1:4]
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
assert len(runs) == 3 * 2 * 4 * 49, len(runs)
half_points = lambda du: round(du * 0.24 * 2)
rows = []
for n in range(len(runs) // 3):
    (body, base), (script, shifted), (_, after) = runs[3 * n:3 * n + 3]
    assert base == after, n
    superscript = n < 4 * 49
    shift = shifted - base if superscript else base - shifted
    rows.append({
        "vertAlign": "superscript" if superscript else "subscript",
        "face": FACES[(n // 49) % 4],
        "sizeHalfPoints": 12 + n % 49,
        "bodyDeviceUnits": body,
        "scriptDeviceUnits": script,
        "scriptHalfPoints": half_points(script),
        "shiftDeviceUnits": shift,
        "shiftHalfPoints": half_points(shift),
    })
sha = lambda path: hashlib.sha256(open(path, "rb").read()).hexdigest()
json.dump({"docxSha256": sha(docx), "pdfSha256": sha(pdf), "deviceUnitPt": 0.24, "rows": rows},
          open(out, "w"), ensure_ascii=False, indent=1)
