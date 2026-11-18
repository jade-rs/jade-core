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

All 60 captures remain **pending**, including the failures. Their presence does
not assert native coordinate agreement. CI checks their original bytes, complete
versioned archive coverage, distinct marker forms, reproducibility and Mac
lineage. Existing required Mac and Windows coordinate gates remain in force.

Word's actual painted PDF distinguishes the XML soft-hyphen element from the
literal Unicode character. At width 65pt the former wraps `electro-` / `magnetic`,
while the latter wraps `electro-mag` / `netic`. A native logical-coordinate dump
cannot by itself prove that an invisible control was not painted as extra ink;
compare the actual PDF painting against the original Word output as well.
