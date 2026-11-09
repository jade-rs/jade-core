# Display math defaults — pending Word measurements

These nine generated DOCX files isolate document math margins and
`dispDef=false` paragraph alignment. No Word PDF or measured coordinates have
been captured yet. These inputs are not evidence of Word parity.

Run `python3 make_probe.py` in this directory to regenerate the deterministic
packages and their SHA-256 manifest (`sources.json`). Each document contains
a short equation with default alignment, an explicitly centered equation, a
long equation with wrapping opportunities, and ordinary text before/after.
The page is 8.5 × 11 inches with one-inch margins (6.5 inches of body width),
with no document grid. Fonts are Cambria and Cambria Math; no font data is
included.

Cases cover absent margins, asymmetric margins, margin elements without values,
an oversized left margin, disabled math defaults with conflicting math layout
settings, alignment from the default paragraph style or document defaults,
an inherited RTL `end` alignment, and a direct alignment overriding its style.
The explicitly centered equation must retain its authored math alignment in
all cases. Do not infer wrapping line positions from unmeasured assumptions.

The properties under test are described in Microsoft's
[default math properties](https://devblogs.microsoft.com/math-in-office/default-math-properties/)
and Open XML documentation for
[left margins](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.math.leftmargin?view=openxml-3.0.1)
and [display defaults](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.math.displaydefaults?view=openxml-3.0.1).

When the desktop is unlocked, export each source with the installed Word:

```sh
osascript export.applescript "$PWD/margins.docx" "$PWD/margins-word.pdf"
```

The script opens only the passed generated source and closes it without
saving changes. It has not been executed for these probes. Record the exact
Word version, source/PDF hashes and installed font hashes with the captured
glyph coordinates before using them as renderer expectations. Compare short
equation origins, explicit alignment, wrap lines, and the final ordinary text
position; preserve any mismatches rather than fitting offsets or tolerances.
