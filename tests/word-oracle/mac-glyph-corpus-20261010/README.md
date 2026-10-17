# Word for Mac glyph corpus

These are the original 118 references used by PRs #439 and #443, supplied on
2026-10-10. The JSON and corresponding Word PDFs are copied without changes.
Every JSON's `docx_sha256` matches `public/fixtures/<name>.docx`, and its
`pdf_sha256` matches `pdf/<name>.pdf`.

Word version: 16.113.3, build 16.113.26092714, macOS 27.0.1. The original
capture directory was `~/Documents/jade-word-capture`. The extra `calib-grid`
PDF in the supplied archive was not part of the 118-reference sweep.

`SHA256.tsv` records each reference's JSON, DOCX and PDF hashes. Do not change
the references to match jade. A new Word capture needs its own provenance.

Run the full corpus after building the playground:

```sh
node tests/playwright/word-glyph-sweep.mjs \
  tests/word-oracle/mac-glyph-corpus-20261010 \
  --platform mac --expected-fixtures 118 --assert-100
```

Both percentages use all 4,107 Word rows as the denominator. A passing subset
does not satisfy the full-corpus gate. Font availability affects measurement;
record which Word fonts are installed when comparing results.
