# Explicit hyphen source forms

`explicit-hyphen-observations.json` retains 60 actual Windows Word captures:
five marker forms × six paragraph widths (35/42/45/48/65/110pt) × document
compatibility modes 14/15. The forms are `<w:softHyphen/>`, literal U+00AD
inside `<w:t>`, `<w:noBreakHyphen/>`, ASCII `-`, and no marker.

Every capture was opened normally, read-only, by Word **16.0.19127.20936**.
The archive and expanded evidence paths include that complete product version.
Capture manifests retain the executable SHA-256, application build, source
identity, original PDF and EMFs. Metadata also records the embedded original
Arial **Version 7.07** font programs and their SHA-256 identities. Compatibility
modes describe document settings; they are not additional installed Word versions.

The unchanged parent is the Mac `explicit-hyphen-wrap-boundaries-mac` original
(Word **16.114.4**). These 60 inputs are derived Windows controls, not new Mac
exports. `generate_windows_explicit_hyphen_probes.py` reproduces all inputs
byte-for-byte. It preserves the section, theme, styles, fonts and every package
part other than `word/document.xml` and `word/settings.xml`.

The raw observations retain **pending** status for full coordinate-baseline
promotion. Their registration alone does not assert native agreement. CI checks
their original bytes, versioned archive coverage, distinct marker forms,
reproducibility and Mac lineage. The additional painted-coordinate gate below
requires actual vector-paint agreement for all 60 cases and 28 mixed controls.

Word's actual painted PDF distinguishes the XML soft-hyphen element from the
literal Unicode character. At width 65pt the former wraps `electro-` / `magnetic`,
while the latter wraps `electro-mag` / `netic`. A native logical-coordinate dump
cannot by itself prove that an invisible control was not painted as extra ink;
compare the actual PDF painting against the original Word output as well.

## Mixed controls and the painted-coordinate gate

`mixed-hyphen-observations.json` adds 28 documents from two independent,
read-only Windows Word captures. The seven patterns combine literal U+00AD
and XML `w:softHyphen` in either order, separate words, a multibyte prefix,
separate runs, and source soft breaks. Widths 45/65pt and compatibility modes 14/15 produce all 28
combinations. `generate_windows_mixed_hyphen_probes.py` reproduces their
original DOCX bytes from the unchanged Mac source.

Both observation sets retain Word 16.0.19127.20936, its executable SHA-256,
original PDF and EMF bytes, capture identity, and original embedded Arial 7.07
program hashes. The Mac parent remains the original Word 16.114.4 capture.
These derived controls are not additional Word-for-Mac exports.

`check_windows_hyphen_paint.py` requires all 88 original cases in Windows CI.
It compares every nonspace painted vector-PDF glyph on both axes at the
existing 0.375pt tolerance, with independent original Word font programs.
There is no alignment or removal of discretionary characters from a logical
dump. When a PDF encodes an actual visible hyphen as U+00AD, the verifier
requires its embedded font's outline and width to equal that font's ASCII
hyphen, then canonicalizes one glyph to one glyph at the unchanged origin.
Missing or duplicate ink, page changes, and X/Y drift remain failures.

The gate separately requires export to preserve live layout and PNG pixels.
Logical dumps intentionally retain editable controls and are not physical
screen-paint traces. The raw observations therefore remain marked `pending`
for full coordinate-baseline promotion. Existing screen/EMF, four-plane font,
and independent Mac coordinate gates remain required.
