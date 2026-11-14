# Windows table style indentation observations

Twenty-eight immutable inputs retain Word 16.0.19127.20936 input/PDF/EMF
bytes, complete executable provenance and capture identity. Observation
coverage is 247 captures / 974 documents. These are measurements for a future
fix; no renderer changes or accepted-coordinate entries are included here.
Mac originals and coordinate tolerances are unchanged.

`20261114T145808245Z-5e0616c6` contains nineteen two-by-two Arial controls.
They isolate `basedOn` inheritance, explicit versus omitted direct indentation,
compatibility modes 12/14/15, and child DXA/nil/auto/percentage overrides.
Fifteen controls already match all four coordinate planes. Four expose missing
style indentation: inherited zero in modes 12/14, a child zero overriding its
parent, and an inherited positive indentation. An ignored child width type
resets its parent's indentation; treating a style's nil as a DXA zero would
regress a currently matching control.

`20261114T150706450Z-bf0874d1` adds nine default-style controls. Each mode
retains a default TableNormal style with zero indentation, removes its
indentation, or removes the styles part and its package declarations entirely.
All three mode-15 controls match. All six mode-12/14 controls expose a 5.4pt
horizontal difference. The default TableNormal case without a declared
indentation and the missing-styles case still use Word's implicit default;
they must remain distinct from a styles part that declares no default table
style. Do not globally shift omitted-indentation tables to fit these inputs.

Measurements used the independently built native renderer with only fonts
extracted from each original Word PDF. Comparisons checked raw EMF, unclipped
Word PDF origins, visible Word PDF origins and separately painted native PDF
origins at the existing 0.375pt absolute X/Y tolerance. No translation or
expected-coordinate input was used by the renderer. None of the twenty-eight
observations is promoted by this data-only change, including the matching
controls. The generator reproduces all input bytes and validates XML parts.
